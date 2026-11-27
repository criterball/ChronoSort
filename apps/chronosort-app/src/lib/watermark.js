// The text watermark, drawn on a canvas at a large size (so it stays sharp on
// big photos) and handed to the backend as a PNG data URL. Used by the viewer's
// Frame and watermark panel and by the batch converter, so both look the same.

const rgbOf = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));

// txt: { text, bold, color: "#rrggbb", outline }. Returns null for empty text.
export function textWatermarkPng(txt) {
  const t = (txt?.text ?? "").trim();
  if (!t) return null;
  const lines = t.split("\n");
  const size = 160, lh = size * 1.2, pad = Math.ceil(size * 0.25);
  const c = document.createElement("canvas");
  const x = c.getContext("2d");
  const font = `${txt.bold ? "700" : "400"} ${size}px "Segoe UI", system-ui, sans-serif`;
  x.font = font;
  const w = Math.ceil(Math.max(...lines.map((l) => x.measureText(l).width))) + 2 * pad;
  c.width = Math.max(1, w);
  c.height = Math.ceil(lines.length * lh) + 2 * pad;
  x.font = font;
  x.textBaseline = "top";
  x.fillStyle = txt.color;
  if (txt.outline) {
    // A soft dark (or light) halo keeps the text readable on any background.
    const light = rgbOf(txt.color).reduce((a, b) => a + b, 0) > 382;
    x.shadowColor = light ? "rgba(0,0,0,0.65)" : "rgba(255,255,255,0.65)";
    x.shadowBlur = size * 0.12;
  }
  lines.forEach((l, n) => x.fillText(l, pad, pad + n * lh + size * 0.05));
  return c.toDataURL("image/png");
}

// The frame and watermark the user last finished with in the viewer (or null).
export const EFFECTS_MEMORY = "chronosort.effects.v1";
export function lastEffects() {
  try {
    return JSON.parse(localStorage.getItem(EFFECTS_MEMORY) ?? "null");
  } catch {
    return null;
  }
}
// The saved settings in the backend's shape.
export function effectsForBackend(fx) {
  return {
    frame: fx.frame, size: +fx.size, color: rgbOf(fx.color), accent: rgbOf(fx.accent),
    watermark: { ...fx.wm, width: +fx.wm.width, margin: +fx.wm.margin, opacity: +fx.wm.opacity },
  };
}
