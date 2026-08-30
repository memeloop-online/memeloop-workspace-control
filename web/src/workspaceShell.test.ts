import assert from "node:assert/strict";
import test from "node:test";
import { reserveWebShellWindow } from "./workspaceShell.ts";

test("reserves one reachable terminal tab and severs its opener", () => {
  const opened = { opener: { name: "console" } } as unknown as Window;
  const calls: unknown[][] = [];
  const target = reserveWebShellWindow((...arguments_) => {
    calls.push(arguments_);
    return opened;
  });

  assert.equal(target, opened);
  assert.equal(opened.opener, null);
  assert.deepEqual(calls, [["about:blank", "_blank"]]);
});

test("preserves the same-tab fallback when a popup is blocked", () => {
  assert.equal(reserveWebShellWindow(() => null), null);
});
