import { rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const bundleDirectory = path.join(root, "target", "release", "bundle", "nsis");

await rm(bundleDirectory, { recursive: true, force: true });
console.log(`Removed stale NSIS bundle output: ${bundleDirectory}`);
