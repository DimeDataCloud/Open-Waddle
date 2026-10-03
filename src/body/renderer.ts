// Draws 16x14 pixel frames with nearest-neighbour scaling. Each frame/palette
// pair is rasterised once into an offscreen canvas and reused.

import type { Palette } from "./palette";
import { FRAMES, SPRITE_H, SPRITE_W, type FrameName } from "./sprites";

export class SpriteRenderer {
  private cache = new Map<string, HTMLCanvasElement>();
  private paletteKey = "";
  private palette: Palette = {};

  constructor(private ctx: CanvasRenderingContext2D, readonly scale: number) {
    ctx.imageSmoothingEnabled = false;
  }

  get width(): number {
    return SPRITE_W * this.scale;
  }

  get height(): number {
    return SPRITE_H * this.scale;
  }

  setPalette(p: Palette): void {
    const key = JSON.stringify(p);
    if (key === this.paletteKey) return;
    this.paletteKey = key;
    this.palette = p;
  }

  private raster(name: FrameName): HTMLCanvasElement {
    const id = `${this.paletteKey}|${name}`;
    let c = this.cache.get(id);
    if (c) return c;
    c = document.createElement("canvas");
    c.width = SPRITE_W;
    c.height = SPRITE_H;
    const g = c.getContext("2d")!;
    FRAMES[name].forEach((row, y) => {
      [...row].forEach((key, x) => {
        const colour = this.palette[key];
        if (key === "." || !colour) return;
        g.fillStyle = colour;
        g.fillRect(x, y, 1, 1);
      });
    });
    if (this.cache.size > 64) this.cache.clear();
    this.cache.set(id, c);
    return c;
  }

  /** Draws a frame with its top-left at (x, y) in canvas pixels. */
  draw(name: FrameName, x: number, y: number, flip: boolean): void {
    const img = this.raster(name);
    const { ctx } = this;
    ctx.save();
    ctx.imageSmoothingEnabled = false;
    if (flip) {
      ctx.translate(Math.round(x) + this.width, Math.round(y));
      ctx.scale(-1, 1);
      ctx.drawImage(img, 0, 0, this.width, this.height);
    } else {
      ctx.drawImage(img, Math.round(x), Math.round(y), this.width, this.height);
    }
    ctx.restore();
  }
}
