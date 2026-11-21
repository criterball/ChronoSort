// Draws the ChronoSort icon (1024x1024 PNG) with no dependencies.
// Usage: node scripts/make-icon.mjs src-tauri/icons/source.png
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const N = 1024;
const px = Buffer.alloc(N * N * 4);

const mix = (a, b, t) => a + (b - a) * t;
const clamp = (v) => Math.max(0, Math.min(1, v));
// Signed distance to a rounded box centred at (0,0).
const roundBox = (x, y, hw, hh, r) => {
  const qx = Math.abs(x) - hw + r, qy = Math.abs(y) - hh + r;
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - r;
};
// Distance to a line segment (for clock hands).
const seg = (x, y, ax, ay, bx, by) => {
  const px_ = x - ax, py = y - ay, dx = bx - ax, dy = by - ay;
  const t = clamp((px_ * dx + py * dy) / (dx * dx + dy * dy));
  return Math.hypot(px_ - dx * t, py - dy * t);
};

function blend(i, r, g, b, a) {
  const o = 1 - a;
  px[i] = r * a + px[i] * o;
  px[i + 1] = g * a + px[i + 1] * o;
  px[i + 2] = b * a + px[i + 2] * o;
  px[i + 3] = Math.min(255, 255 * a + px[i + 3] * o);
}

for (let yy = 0; yy < N; yy++) {
  for (let xx = 0; xx < N; xx++) {
    const i = (yy * N + xx) * 4;
    const x = xx - N / 2 + 0.5, y = yy - N / 2 + 0.5;
    // background tile: deep indigo -> teal gradient
    const dBg = roundBox(x, y, 460, 460, 200);
    const aBg = clamp(0.5 - dBg);
    if (aBg > 0) {
      const t = clamp((xx + yy) / (2 * N));
      blend(i, mix(32, 18, t), mix(46, 132, t), mix(110, 140, t), aBg);
    }
    // clock face ring
    const r = Math.hypot(x, y);
    const ring = Math.abs(r - 270) - 34;
    blend(i, 255, 255, 255, clamp(0.5 - ring) * 0.95);
    // 12 tick marks
    const ang = Math.atan2(y, x);
    const k = Math.round(ang / (Math.PI / 6)) * (Math.PI / 6);
    const tick = seg(x, y, Math.cos(k) * 180, Math.sin(k) * 180, Math.cos(k) * 205, Math.sin(k) * 205) - 9;
    blend(i, 255, 255, 255, clamp(0.5 - tick) * 0.8);
    // hands: hour (to ~10 o'clock) and minute (to 12) + warm accent "year" hand
    const hHour = seg(x, y, 0, 0, -120, -70) - 22;
    const hMin = seg(x, y, 0, 0, 0, -190) - 16;
    blend(i, 255, 255, 255, clamp(0.5 - hHour));
    blend(i, 255, 255, 255, clamp(0.5 - hMin));
    const hYear = seg(x, y, 0, 0, 150, 110) - 12;
    blend(i, 255, 190, 92, clamp(0.5 - hYear));
    const hub = r - 34;
    blend(i, 255, 190, 92, clamp(0.5 - hub));
  }
}

// --- PNG encoding ---
const crcTable = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
};
const raw = Buffer.alloc(N * (N * 4 + 1));
for (let y = 0; y < N; y++) {
  raw[y * (N * 4 + 1)] = 0;
  px.copy(raw, y * (N * 4 + 1) + 1, y * N * 4, (y + 1) * N * 4);
}
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(N, 0);
ihdr.writeUInt32BE(N, 4);
ihdr.set([8, 6, 0, 0, 0], 8);
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync(process.argv[2] ?? "icon-source.png", png);
console.log("icon written");
