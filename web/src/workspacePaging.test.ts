import assert from "node:assert/strict";
import test from "node:test";
import { previousWorkspaceCursor } from "./workspacePaging.ts";

test("cursor history returns the cursor that starts the previous page", () => {
  const history = [null, "after-page-1", "after-page-2"];
  assert.equal(previousWorkspaceCursor(history, 3), "after-page-1");
  assert.equal(previousWorkspaceCursor(history.slice(0, 2), 2), null);
});

test("first page has no previous cursor", () => {
  assert.equal(previousWorkspaceCursor([null], 1), null);
  assert.equal(previousWorkspaceCursor([], 0), null);
});

