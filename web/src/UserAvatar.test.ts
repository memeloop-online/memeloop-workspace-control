import assert from "node:assert/strict";
import test from "node:test";
import { avatarHue, initials } from "./userIdentity.ts";

test("builds useful fallback initials for single and multi-word names", () => {
  assert.equal(initials("Lin Dongwu"), "LD");
  assert.equal(initials("林东吴"), "林东");
  assert.equal(initials("  Meme   Loop  "), "ML");
  assert.equal(initials(""), "?");
});

test("assigns a stable avatar color without exposing user data", () => {
  assert.equal(avatarHue("user-123"), avatarHue("user-123"));
  assert.ok(avatarHue("user-123") >= 0 && avatarHue("user-123") < 360);
  assert.notEqual(avatarHue("user-123"), avatarHue("user-124"));
});
