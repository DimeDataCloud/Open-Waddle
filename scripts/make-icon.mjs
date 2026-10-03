// Renders Waddle's idle frame to a 1024x1024 PNG for `tauri icon`.
// Usage: node scripts/make-icon.mjs && npx tauri icon src-tauri/icons/source.png
import { readFileSync, writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";

const sprites = JSON.parse(readFileSync(new URL("../src/body/waddle.sprites.json", import.meta.url)));
const palette = {
  o: [42, 30, 20], b: [255, 210, 63], d: [158, 130, 39], k: [92, 76, 23],
  w: [255, 255, 255], e: [20, 20, 28], a: [255, 140, 26], A: [158, 87, 16],
};
const frame = sprites.frames.idle;
const size = 1024, scale = 56;
const ox = Math.floor((size - 16 * scale) / 2), oy = Math.floor((size - 14 * scale) / 2);
const raw = Buffer.alloc((size * 4 + 1) * size);
for (let y = 0; y < size; y++) {
  raw[y * (size * 4 + 1)] = 0;
  for (let x = 0; x < size; x++) {
    const sx = Math.floor((x - ox) / scale), sy = Math.floor((y - oy) / scale);
    const key = sx >= 0 && sx < 16 && sy >= 0 && sy < 14 ? frame[sy][sx] : ".";
    const c = palette[key];
    const i = y * (size * 4 + 1) + 1 + x * 4;
    if (c) { raw[i] = c[0]; raw[i + 1] = c[1]; raw[i + 2] = c[2]; raw[i + 3] = 255; }
  }
}
const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc = (buf) => { let c = 0xffffffff; for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
const chunk = (type, data) => {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const c = Buffer.alloc(4); c.writeUInt32BE(crc(td));
  return Buffer.concat([len, td, c]);
};
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4); ihdr[8] = 8; ihdr[9] = 6;
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw, { level: 9 })), chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync(new URL("../src-tauri/icons/source.png", import.meta.url), png);
console.log("wrote src-tauri/icons/source.png");
