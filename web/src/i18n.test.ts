import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const localeNames = ["zh-CN", "en", "ru"] as const;

function loadLocale(name: (typeof localeNames)[number]): Record<string, string> {
  return JSON.parse(readFileSync(new URL(`./locales/${name}.json`, import.meta.url), "utf8")) as Record<string, string>;
}

test("all supported locales expose the same message keys", () => {
  const keySets = localeNames.map((name) => [name, Object.keys(loadLocale(name)).sort()] as const);
  const [, firstKeys] = keySets[0]!;
  for (const [name, keys] of keySets.slice(1)) {
    assert.deepEqual(keys, firstKeys, `${name} locale keys differ from zh-CN`);
  }
});

test("locale messages are strings", () => {
  for (const name of localeNames) {
    for (const [key, value] of Object.entries(loadLocale(name))) {
      assert.equal(typeof value, "string", `${name}.${key} must be a string`);
    }
  }
});
