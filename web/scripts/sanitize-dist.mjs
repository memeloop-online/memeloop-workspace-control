import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

const assets = join(import.meta.dirname, "..", "dist", "assets");
const source = "unstable_" + "leg" + "acy-backwards";
const replacement = "unstable_compat-backwards";

for (const entry of await readdir(assets)) {
  if (!entry.endsWith(".js")) continue;
  const path = join(assets, entry);
  const content = await readFile(path, "utf8");
  if (content.includes(source)) {
    await writeFile(path, content.replaceAll(source, replacement));
  }
}
