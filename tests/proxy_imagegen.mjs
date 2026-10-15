import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, writeFile, readFile, readdir, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';

const script = resolve('skills/proxy-imagegen/scripts/generate.sh');
const image = Buffer.from('89504e470d0a1a0a00010203', 'hex');
const result = image.toString('base64');
const item = { type: 'image_generation_call', result };
const completed = (output = [item]) => ({ type: 'response.completed', response: { status: 'completed', output } });
const event = (value) => `data: ${JSON.stringify(value)}\r\n\r\n`;

async function fixture(t, handler, options = {}) {
  const root = await mkdtemp(join(tmpdir(), 'mwc-imagegen-test-'));
  const temporary = join(root, 'temporary');
  const bin = join(root, 'bin');
  await mkdir(temporary); await mkdir(bin);
  const secret = 'test-key-that-must-never-be-logged';
  for (const name of ['python', 'python3', 'pwsh']) {
    await writeFile(join(bin, name), '#!/bin/sh\necho FORBIDDEN_RUNTIME >&2\nexit 97\n', { mode: 0o755 });
  }
  const requests = [];
  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    try {
      assert.equal(request.url, '/active/responses');
      assert.equal(request.headers.authorization, `Bearer ${secret}`);
      const body = JSON.parse(Buffer.concat(chunks));
      assert.equal(body.model, 'configured-model');
      assert.equal(body.input, '生成一张图片，保持 "文字" 和换行\nsecond line');
      assert.deepEqual(body.tools, [{ type: 'image_generation', model: 'gpt-image-2' }]);
      assert.deepEqual(body.tool_choice, { type: 'image_generation' });
      assert.equal(body.stream, true); assert.equal(body.store, false);
      requests.push(body);
      await handler(response, requests.length, { root, secret, temporary });
    } catch (error) {
      response.writeHead(500); response.end('fixture assertion failed');
      server.fixtureError = error;
    }
  });
  await new Promise((done) => server.listen(0, '127.0.0.1', done));
  const url = `http://127.0.0.1:${server.address().port}/active`;
  const config = options.literal ?
    `model = 'configured-model'\nmodel_provider = 'selected-provider'\n[model_providers.other]\nbase_url = 'http://127.0.0.1:1/wrong'\n[model_providers.'selected-provider']\nbase_url = '${url}' # comment\n` :
    `model = "configured-model"\nmodel_provider = "selected-provider"\n[model_providers.other]\nbase_url = "http://127.0.0.1:1/wrong"\n[model_providers."selected-provider"]\nbase_url = "${url}" # comment\n`;
  await writeFile(join(root, 'config.toml'), config);
  await writeFile(join(root, 'auth.json'), JSON.stringify({ OPENAI_API_KEY: secret }));
  t.after(async () => {
    server.closeAllConnections();
    await new Promise((done) => server.close(done));
    await rm(root, { recursive: true, force: true });
  });
  return {
    root, requests,
    async run(extra = [], timeoutMs = 30_000) {
      const started = Date.now();
      const child = spawn('bash', [script, '--prompt', '生成一张图片，保持 "文字" 和换行\nsecond line',
        '--out', join(root, 'output with spaces', 'image.png'), '--timeout', '5', ...extra], {
        env: { ...process.env, CODEX_HOME: root, TMPDIR: temporary, PATH: `${bin}:${process.env.PATH}` },
        stdio: ['ignore', 'pipe', 'pipe'],
      });
      let stdout = ''; let stderr = '';
      child.stdout.on('data', (data) => { stdout += data; });
      child.stderr.on('data', (data) => { stderr += data; });
      const timer = setTimeout(() => child.kill('SIGTERM'), timeoutMs);
      const code = await new Promise((done) => child.on('exit', done));
      clearTimeout(timer);
      assert.ok(!stdout.includes(secret) && !stderr.includes(secret), 'credential leaked to terminal');
      assert.ok(!stderr.includes('FORBIDDEN_RUNTIME'), 'unexpected Python/PowerShell dependency');
      assert.deepEqual(await readdir(temporary), [], 'temporary credentials or response left behind');
      if (server.fixtureError) throw server.fixtureError;
      return { code, stdout, stderr, elapsed: Date.now() - started };
    },
    async assertImage() { assert.deepEqual(await readFile(join(root, 'output with spaces', 'image.png')), image); },
  };
}

test('active provider, UTF-8 prompt, CRLF SSE, no Python, and private curl credentials', async (t) => {
  const f = await fixture(t, async (response, _attempt, context) => {
    const [directory] = await readdir(context.temporary);
    assert.equal((await stat(join(context.temporary, directory, 'curl.conf'))).mode & 0o777, 0o600);
    const pid = (await readFile(join(context.temporary, directory, 'curl.pid'), 'utf8')).trim();
    const argv = await readFile(`/proc/${pid}/cmdline`, 'utf8');
    assert.ok(!argv.includes(context.secret));
    response.writeHead(200, { 'Content-Type': 'text/event-stream' });
    response.end(': keep-alive\r\n\r\nevent: image\r\n' + event(completed()));
  });
  assert.equal((await f.run()).code, 0); await f.assertImage();
  assert.equal(f.requests.length, 1);
});

test('single quoted TOML strings and completed output_item fallback', async (t) => {
  const f = await fixture(t, (r) => r.end(event({ type: 'response.output_item.done', item }) + event(completed([]))), { literal: true });
  assert.equal((await f.run()).code, 0); await f.assertImage();
});

test('stops on completed even when the SSE connection stays open', async (t) => {
  const f = await fixture(t, (r) => { r.writeHead(200); r.write(event(completed())); });
  const r = await f.run(); assert.equal(r.code, 0); assert.ok(r.elapsed < 4500); await f.assertImage();
});

test('multiline data event, EOF delimiter, data URI and JPEG support', async (t) => {
  const jpeg = Buffer.from('ffd8ff000102', 'hex');
  const value = completed([{ ...item, result: `data:image/jpeg;base64,${jpeg.toString('base64')}` }]);
  const payload = JSON.stringify(value, null, 2).split('\n').map((line) => `data: ${line}`).join('\n');
  const f = await fixture(t, (r) => r.end(payload));
  assert.equal((await f.run()).code, 0);
  assert.deepEqual(await readFile(join(f.root, 'output with spaces', 'image.png')), jpeg);
});

test('HTTP 401 reports status without echoing upstream secrets and never retries', async (t) => {
  const f = await fixture(t, (r, _n, { secret }) => { r.writeHead(401); r.end(`invalid ${secret}`); });
  const r = await f.run(); assert.equal(r.code, 1); assert.match(r.stderr, /HTTP 401/);
  assert.equal(f.requests.length, 1);
});

test('HTTP 429/500 retry then success within the configured attempt count', async (t) => {
  const f = await fixture(t, (r, n) => {
    if (n < 3) { r.writeHead(n === 1 ? 429 : 500); r.end('retry'); }
    else r.end(event(completed()));
  });
  assert.equal((await f.run()).code, 0); assert.equal(f.requests.length, 3); await f.assertImage();
});

test('a preview without completed is retried, not saved as a final image', async (t) => {
  const f = await fixture(t, (r) => r.end(event({ type: 'response.output_item.done', item })));
  const r = await f.run(['--attempts', '2']); assert.equal(r.code, 1);
  assert.equal(f.requests.length, 2);
  await assert.rejects(readFile(join(f.root, 'output with spaces', 'image.png')), { code: 'ENOENT' });
});

test('SSE error retries; default attempts stops at three', async (t) => {
  const f = await fixture(t, (r) => r.end(event({ type: 'response.failed', response: { error: { message: 'hidden' } } })));
  const r = await f.run(); assert.equal(r.code, 1); assert.equal(f.requests.length, 3);
});

for (const invalid of ['not valid base64!', Buffer.from('not an image').toString('base64'), '']) {
  test(`rejects invalid image bytes and preserves an existing output (${invalid.length})`, async (t) => {
    const f = await fixture(t, (r) => r.end(event(completed([{ ...item, result: invalid }]))));
    await mkdir(join(f.root, 'output with spaces'));
    await writeFile(join(f.root, 'output with spaces', 'image.png'), 'previous file');
    assert.equal((await f.run(['--attempts', '1'])).code, 1);
    assert.equal(await readFile(join(f.root, 'output with spaces', 'image.png'), 'utf8'), 'previous file');
  });
}

test('no connection fails promptly and cleans private files', async (t) => {
  const f = await fixture(t, (r) => r.end(event(completed())));
  await writeFile(join(f.root, 'config.toml'), 'model="configured-model"\nmodel_provider="offline"\n[model_providers.offline]\nbase_url="http://127.0.0.1:1"\n');
  const r = await f.run(['--attempts', '1']); assert.equal(r.code, 1);
  assert.ok(r.elapsed < 4500); assert.equal(f.requests.length, 0);
});

test('unsupported config fails before making a request', async (t) => {
  const f = await fixture(t, (r) => r.end(event(completed())));
  await writeFile(join(f.root, 'config.toml'), 'model=42\nmodel_provider="other"\n');
  const r = await f.run(); assert.equal(r.code, 1); assert.equal(f.requests.length, 0);
  assert.match(r.stderr, /TOML/);
});

// Old installed skill instructions can still execute the previous path.
test('old executor path delegates to the Bash executor', async () => {
  const child = spawn('bash', [resolve('skills/proxy-imagegen/scripts/generate.ps1'), '--help']);
  let text = ''; child.stdout.on('data', (d) => { text += d; });
  assert.equal(await new Promise((done) => child.on('exit', done)), 0);
  assert.match(text, /No Python is required/);
});
