// Runtime palette maths: every shade is derived from one base colour, so any
// user-picked colour keeps the same depth and contrast (blueprint: S_dark =
// base x 0.62, S_darkest = base x 0.36).

export type RGB = [number, number, number];

export const DARK = 0.62;
export const DARKEST = 0.36;

export function hexToRgb(hex: string): RGB {
  let h = hex.trim().replace(/^#/, "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  if (!/^[0-9a-fA-F]{6}$/.test(h)) throw new Error(`not a hex colour: ${hex}`);
  const n = parseInt(h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function rgbToHex([r, g, b]: RGB): string {
  return "#" + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");
}

export function shade(c: RGB, k: number): RGB {
  return [c[0] * k, c[1] * k, c[2] * k].map((v) => Math.max(0, Math.min(255, Math.round(v)))) as RGB;
}

export type Palette = Record<string, string>;

const OUTLINE = "#2a1e14";
const EYE = "#14141c";
const ACCENT = "#ff8c1a";
const ALARM = "#ff5a4f";

/** Sprite palette keys (see waddle.sprites.json legend) → CSS colours. */
export function buildPalette(baseHex: string, accentHex = ACCENT): Palette {
  let base: RGB;
  try {
    base = hexToRgb(baseHex);
  } catch {
    base = hexToRgb("#FFD23F");
  }
  const accent = hexToRgb(accentHex);
  return {
    o: OUTLINE,
    b: rgbToHex(base),
    d: rgbToHex(shade(base, DARK)),
    k: rgbToHex(shade(base, DARKEST)),
    w: "#ffffff",
    e: EYE,
    a: rgbToHex(accent),
    A: rgbToHex(shade(accent, DARK)),
  };
}

/** The red "needs your approval" look used while a tier 3 action waits. */
export function alarmPalette(accentHex = ACCENT): Palette {
  return buildPalette(ALARM, accentHex);
}
