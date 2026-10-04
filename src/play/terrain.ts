// Destructible terrain for play mode: the screen copy split into 4-px cells.
// Windows are solid ground the duck can stand on and blast holes into; the
// wallpaper around them is open air. The bottom edge of the screen is a floor.

export const CELL = 4;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export class Terrain {
  readonly cols: number;
  readonly rows: number;
  /** 1 = intact solid cell, 0 = air or blasted away. */
  readonly solid: Uint8Array;
  readonly total: number;
  private left: number;

  constructor(
    readonly width: number,
    readonly height: number,
    rects: Rect[],
  ) {
    this.cols = Math.ceil(width / CELL);
    this.rows = Math.ceil(height / CELL);
    this.solid = new Uint8Array(this.cols * this.rows);
    for (const r of rects) {
      const c0 = Math.max(0, Math.floor(r.x / CELL));
      const c1 = Math.min(this.cols, Math.ceil((r.x + r.w) / CELL));
      const r0 = Math.max(0, Math.floor(r.y / CELL));
      const r1 = Math.min(this.rows, Math.ceil((r.y + r.h) / CELL));
      for (let row = r0; row < r1; row++) this.solid.fill(1, row * this.cols + c0, row * this.cols + Math.max(c0, c1));
    }
    this.total = this.solid.reduce((n, v) => n + v, 0);
    this.left = this.total;
  }

  /** Share of the original solid cells that have been blasted (0..1). */
  get destroyed(): number {
    return this.total === 0 ? 0 : 1 - this.left / this.total;
  }

  /** Solid at a point? Off the sides and below the floor count as solid; above the top is air. */
  solidAt(x: number, y: number): boolean {
    if (y >= this.height || x < 0 || x >= this.width) return y >= 0;
    if (y < 0) return false;
    return this.solid[Math.floor(y / CELL) * this.cols + Math.floor(x / CELL)] === 1;
  }

  /** Whether any solid cell (or the screen edge) overlaps the box. */
  hitsBox(x: number, y: number, w: number, h: number): boolean {
    if (x < 0 || x + w > this.width || y + h > this.height) return true;
    const c0 = Math.floor(x / CELL);
    const c1 = Math.floor((x + w - 0.01) / CELL);
    const r0 = Math.max(0, Math.floor(y / CELL));
    const r1 = Math.floor((y + h - 0.01) / CELL);
    for (let r = r0; r <= r1; r++) {
      const row = r * this.cols;
      for (let c = c0; c <= c1; c++) if (this.solid[row + c] === 1) return true;
    }
    return false;
  }

  /** First solid point along a segment (checked every 2 px), or null. Screen edges don't count. */
  firstHit(x0: number, y0: number, x1: number, y1: number): { x: number; y: number } | null {
    const steps = Math.max(1, Math.ceil(Math.hypot(x1 - x0, y1 - y0) / 2));
    for (let i = 1; i <= steps; i++) {
      const x = x0 + ((x1 - x0) * i) / steps;
      const y = y0 + ((y1 - y0) * i) / steps;
      if (x < 0 || y < 0 || x >= this.width || y >= this.height) continue;
      if (this.solid[Math.floor(y / CELL) * this.cols + Math.floor(x / CELL)] === 1) return { x, y };
    }
    return null;
  }

  /** Blasts a disc. Calls `gone` with the centre of each cell destroyed; returns how many. */
  carveCircle(cx: number, cy: number, r: number, gone?: (x: number, y: number) => void): number {
    let n = 0;
    const c0 = Math.max(0, Math.floor((cx - r) / CELL));
    const c1 = Math.min(this.cols - 1, Math.floor((cx + r) / CELL));
    const r0 = Math.max(0, Math.floor((cy - r) / CELL));
    const r1 = Math.min(this.rows - 1, Math.floor((cy + r) / CELL));
    for (let row = r0; row <= r1; row++) {
      for (let col = c0; col <= c1; col++) {
        const i = row * this.cols + col;
        if (this.solid[i] !== 1) continue;
        const x = col * CELL + CELL / 2;
        const y = row * CELL + CELL / 2;
        if ((x - cx) ** 2 + (y - cy) ** 2 > r * r) continue;
        this.solid[i] = 0;
        n++;
        gone?.(x, y);
      }
    }
    this.left -= n;
    return n;
  }

  /** Blasts a thick line (laser). */
  carveLine(x0: number, y0: number, x1: number, y1: number, halfWidth: number, gone?: (x: number, y: number) => void): number {
    const steps = Math.max(1, Math.ceil(Math.hypot(x1 - x0, y1 - y0) / CELL));
    let n = 0;
    for (let i = 0; i <= steps; i++) n += this.carveCircle(x0 + ((x1 - x0) * i) / steps, y0 + ((y1 - y0) * i) / steps, halfWidth, gone);
    return n;
  }

  /** A random intact cell centre inside `area` (or anywhere), for the bot to aim at. */
  randomSolid(rand: () => number, area?: Rect): { x: number; y: number } | null {
    const a = area ?? { x: 0, y: 0, w: this.width, h: this.height };
    for (let tries = 0; tries < 400; tries++) {
      const x = a.x + rand() * a.w;
      const y = a.y + rand() * a.h;
      if (y >= 0 && y < this.height && this.solidAt(x, y)) return { x, y };
    }
    return null;
  }
}
