import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(root, "src-tauri/icons/icon.ico.base64");
const icoTarget = resolve(root, "src-tauri/icons/icon.ico");
const pngTarget = resolve(root, "src-tauri/icons/icon.png");
const pngSignature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

const encoded = (await readFile(source, "utf8")).replace(/\s+/g, "");
if (!encoded) {
  throw new Error("HomeServer icon source is empty");
}

const ico = Buffer.from(encoded, "base64");
if (
  ico.length < 6 ||
  ico.readUInt16LE(0) !== 0 ||
  ico.readUInt16LE(2) !== 1
) {
  throw new Error("HomeServer icon source is not a valid ICO container");
}

const imageCount = ico.readUInt16LE(4);
if (imageCount < 1 || ico.length < 6 + imageCount * 16) {
  throw new Error("HomeServer ICO does not contain a complete image directory");
}

const pngImages = [];
for (let index = 0; index < imageCount; index += 1) {
  const entryOffset = 6 + index * 16;
  const width = ico[entryOffset] || 256;
  const height = ico[entryOffset + 1] || 256;
  const byteLength = ico.readUInt32LE(entryOffset + 8);
  const imageOffset = ico.readUInt32LE(entryOffset + 12);
  const imageEnd = imageOffset + byteLength;

  if (
    byteLength < pngSignature.length ||
    imageOffset < 6 + imageCount * 16 ||
    imageEnd > ico.length
  ) {
    continue;
  }

  const image = ico.subarray(imageOffset, imageEnd);
  if (image.subarray(0, pngSignature.length).equals(pngSignature)) {
    pngImages.push({ area: width * height, image });
  }
}

if (pngImages.length === 0) {
  throw new Error("HomeServer ICO does not contain an embedded PNG icon");
}

pngImages.sort((left, right) => right.area - left.area);
await mkdir(dirname(icoTarget), { recursive: true });
await writeFile(icoTarget, ico);
await writeFile(pngTarget, pngImages[0].image);
console.log(`Prepared ${icoTarget}`);
console.log(`Prepared ${pngTarget}`);
