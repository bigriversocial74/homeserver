import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(root, "src-tauri/icons/icon.ico.base64");
const target = resolve(root, "src-tauri/icons/icon.ico");

const encoded = (await readFile(source, "utf8")).replace(/\s+/g, "");
if (!encoded) {
  throw new Error("HomeServer icon source is empty");
}

await mkdir(dirname(target), { recursive: true });
await writeFile(target, Buffer.from(encoded, "base64"));
console.log(`Prepared ${target}`);
