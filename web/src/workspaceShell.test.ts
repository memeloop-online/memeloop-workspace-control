import assert from "node:assert/strict";
import test from "node:test";
import { reserveWebShellWindow } from "./workspaceShell.ts";

test("reserves one reachable terminal tab and severs its opener", () => {
  const status = { attributes: {} as Record<string, string>, style: {} as Record<string, string>, textContent: "" };
  const appended: unknown[] = [];
  const opened = {
    opener: { name: "console" },
    document: {
      title: "",
      createElement: () => ({ ...status, setAttribute(name: string, value: string) { status.attributes[name] = value; }, style: { set cssText(value: string) { status.style.cssText = value; } } }),
      body: { replaceChildren() {}, append(node: unknown) { appended.push(node); } },
    },
  } as unknown as Window;
  const calls: unknown[][] = [];
  const target = reserveWebShellWindow((...arguments_: unknown[]) => {
    calls.push(arguments_);
    return opened;
  }, "Preparing connection…");

  assert.equal(target, opened);
  assert.equal(opened.opener, null);
  assert.deepEqual(calls, [["about:blank", "_blank"]]);
  assert.equal(opened.document.title, "Preparing connection…");
  assert.equal((appended[0] as { textContent: string }).textContent, "Preparing connection…");
});

test("leaves the reserved tab untouched without a loading label", () => {
  const opened = { opener: null, document: { title: "" } } as unknown as Window;
  const target = reserveWebShellWindow(() => opened);
  assert.equal(target, opened);
  assert.equal(opened.document.title, "");
});

test("preserves the same-tab fallback when a popup is blocked", () => {
  assert.equal(reserveWebShellWindow(() => null), null);
});
