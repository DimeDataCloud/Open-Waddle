import { describe, expect, it } from "vitest";
import { alarmPalette, buildPalette, hexToRgb, rgbToHex, shade } from "./palette";

describe("palette", () => {
  it("parses and formats hex colours", () => {
    expect(hexToRgb("#FFD23F")).toEqual([255, 210, 63]);
    expect(hexToRgb("fff")).toEqual([255, 255, 255]);
    expect(rgbToHex([255, 210, 63])).toBe("#ffd23f");
    expect(() => hexToRgb("nope")).toThrow();
  });

  it("derives shadows as 0.62 and 0.36 of the base", () => {
    const p = buildPalette("#FFD23F");
    expect(p.b).toBe("#ffd23f");
    expect(p.d).toBe(rgbToHex(shade([255, 210, 63], 0.62)));
    expect(p.d).toBe("#9e8227");
    expect(p.k).toBe("#5c4c17");
  });

  it("falls back to Waddle yellow on a bad colour and has an alarm variant", () => {
    expect(buildPalette("garbage").b).toBe("#ffd23f");
    expect(alarmPalette().b).not.toBe(buildPalette("#FFD23F").b);
  });
});
