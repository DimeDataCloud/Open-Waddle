// Play mode: a frozen copy of the screen becomes a destructible level. The duck
// runs, jumps and flaps across the windows and blasts them apart, revealing a
// pixel-art pond underneath. The real desktop is never touched; Esc ends it.

import type { Palette } from "../body/palette";
import { SpriteRenderer } from "../body/renderer";
import type { FrameName } from "../body/sprites";
import { Bot } from "./bot";
import { Player, type Controls } from "./player";
import { drawPond } from "./pond";
import { Sfx } from "./sound";
import { CELL, Terrain, type Rect } from "./terrain";
import { fire, GRENADE, steer, WEAPONS, weaponIndex, type Projectile, type Weapon } from "./weapons";

export interface PlayOptions {
  /** PNG data URL of the screen under the overlay, at the overlay's size. */
  image: string;
  /** Window rectangles in overlay coordinates: the solid ground. */
  windows: Rect[];
  /** Where the duck is standing now (feet: centre x, bottom y). */
  duck: { x: number; y: number };
  palette: Palette;
  autoplay: boolean;
  /** A window the AI chose to aim at. */
  target: Rect | null;
  weapon: string | null;
  /** Called after the game closes, with where the duck ended up (feet). */
  onExit: (feet: { x: number; y: number }, destroyed: number) => void;
}

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  max: number;
  size: number;
  colour: string;
  /** Debris settles on what's left of the windows; smoke and sparks don't. */
  debris: boolean;
}

interface Flash {
  x: number;
  y: number;
  r: number;
  life: number;
  max: number;
}

interface Beam {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  life: number;
}

const SCALE = 3;
const MAX_PARTICLES = 2500;
const MILESTONES: [number, string][] = [
  [0.1, "Quack attack!"],
  [0.25, "Feathers flying!"],
  [0.5, "Half the desktop is pond!"],
  [0.75, "QUACKTASTIC!"],
  [0.95, "Total duckstruction!"],
];
const AUTOPLAY_SECONDS = 45;
/** With nobody at the keyboard, close by itself so the screen is never left covered. */
const IDLE_EXIT_SECONDS = 25;

function seeded(seed: number): () => number {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return (s >>> 0) / 4294967296;
  };
}

function canvas(root: HTMLElement, w: number, h: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  c.style.cssText = "position:absolute;inset:0;width:100%;height:100%;image-rendering:pixelated";
  root.appendChild(c);
  return c;
}

export class Game {
  private root: HTMLDivElement;
  private shot!: HTMLCanvasElement;
  private shotCtx!: CanvasRenderingContext2D;
  private fx!: CanvasRenderingContext2D;
  private original!: HTMLImageElement;
  private pixels!: Uint8ClampedArray;
  private terrain!: Terrain;
  private player!: Player;
  private sprite!: SpriteRenderer;
  private bot: Bot | null = null;
  private sfx = new Sfx();
  private rand = seeded(Date.now());
  private w: number;
  private h: number;

  private projectiles: Projectile[] = [];
  private particles: Particle[] = [];
  private flashes: Flash[] = [];
  private beams: Beam[] = [];
  private keys = new Set<string>();
  private jumpQueued = false;
  private mouse = { x: 0, y: 0, down: false };
  private weapon = 0;
  private cooldown = 0;
  private grenadeCooldown = 0;
  private shake = 0;
  private toast: { text: string; life: number } | null = null;
  private milestone = 0;
  private lastShot = 0;
  private elapsed = 0;
  private idle = 0;
  private running = false;
  private last = 0;
  private exitBox = { x: 0, y: 0, w: 0, h: 0 };
  private cleanup: (() => void)[] = [];

  constructor(private opts: PlayOptions) {
    this.w = window.innerWidth;
    this.h = window.innerHeight;
    this.root = document.createElement("div");
    this.root.id = "play";
    this.root.style.cssText = "position:fixed;inset:0;z-index:5;cursor:crosshair;overflow:hidden;outline:none";
    this.root.tabIndex = -1;
    this.weapon = weaponIndex(opts.weapon);
  }

  async start(): Promise<void> {
    const { w, h } = this;
    const under = canvas(this.root, w, h);
    drawPond(under, w, h, Math.floor(this.rand() * 1e6));
    this.shot = canvas(this.root, w, h);
    this.shotCtx = this.shot.getContext("2d", { willReadFrequently: true })!;
    const fxCanvas = canvas(this.root, w, h);
    this.fx = fxCanvas.getContext("2d")!;
    this.fx.imageSmoothingEnabled = false;
    this.sprite = new SpriteRenderer(this.fx, SCALE);
    this.sprite.setPalette(this.opts.palette);

    this.original = new Image();
    this.original.src = this.opts.image;
    await this.original.decode();
    document.body.appendChild(this.root);
    this.reset();

    const feet = this.opts.duck;
    this.player = new Player(feet.x - this.sprite.width / 2, feet.y - this.sprite.height, this.sprite.width, this.sprite.height);
    // Never start buried inside a window: pop up onto the nearest ledge above,
    // or failing that, clear a pocket around the duck.
    const pl = this.player;
    if (this.terrain.hitsBox(pl.x, pl.y, pl.w, pl.h)) {
      let y = pl.y;
      while (y > 0 && this.terrain.hitsBox(pl.x, y, pl.w, pl.h)) y -= CELL;
      if (!this.terrain.hitsBox(pl.x, y, pl.w, pl.h)) pl.y = y;
      else this.blast(pl.cx, pl.cy, Math.max(pl.w, pl.h) * 0.8, false);
    }
    if (this.opts.autoplay) this.bot = new Bot(this.rand, this.opts.target);
    if (this.opts.target) this.say("Target locked!");
    this.listen();
    this.root.focus();
    this.running = true;
    this.last = performance.now();
    requestAnimationFrame((t) => this.frame(t));
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;
    for (const off of this.cleanup) off();
    this.root.remove();
    const p = this.player;
    this.opts.onExit({ x: p.cx, y: p.y + p.h }, this.terrain.destroyed);
  }

  /** Puts every pixel back: the copy is whole again. */
  private reset(): void {
    this.shotCtx.globalCompositeOperation = "source-over";
    this.shotCtx.clearRect(0, 0, this.w, this.h);
    this.shotCtx.drawImage(this.original, 0, 0, this.w, this.h);
    this.pixels = this.shotCtx.getImageData(0, 0, this.w, this.h).data;
    this.terrain = new Terrain(this.w, this.h, this.ground());
    this.projectiles = [];
    this.particles = [];
    this.milestone = 0;
  }

  /** Windows are the ground. With none open (or none known), the whole screen
   *  below a strip of sky is fair game, so there's always something to wreck. */
  private ground(): Rect[] {
    const big = this.opts.windows.filter((r) => r.w >= 40 && r.h >= 40);
    if (big.length) return big;
    const top = Math.round(this.h * 0.3);
    return [{ x: 0, y: top, w: this.w, h: this.h - top }];
  }

  // ---------- input ----------

  private listen(): void {
    const on = <K extends keyof WindowEventMap>(type: K, fn: (e: WindowEventMap[K]) => void, opts?: AddEventListenerOptions) => {
      window.addEventListener(type, fn, opts);
      this.cleanup.push(() => window.removeEventListener(type, fn, opts));
    };
    on("keydown", (e) => {
      this.takeOver();
      const k = e.key.toLowerCase();
      if ([" ", "arrowup", "arrowdown", "arrowleft", "arrowright"].includes(k)) e.preventDefault();
      if (k === "escape") return this.stop();
      if (!this.keys.has(k) && (k === " " || k === "w" || k === "arrowup")) this.jumpQueued = true;
      this.keys.add(k);
      if (k >= "1" && k <= String(WEAPONS.length)) this.pick(Number(k) - 1);
      if (k === "r") this.reset();
      if (k === "m") {
        this.sfx.muted = !this.sfx.muted;
        this.say(this.sfx.muted ? "Sound off" : "Sound on");
      }
    });
    on("keyup", (e) => this.keys.delete(e.key.toLowerCase()));
    on("blur", () => this.keys.clear());
    on("pointermove", (e) => {
      this.mouse.x = e.clientX;
      this.mouse.y = e.clientY;
      this.idle = 0;
    });
    on("pointerdown", (e) => {
      const b = this.exitBox;
      if (e.clientX >= b.x && e.clientX <= b.x + b.w && e.clientY >= b.y && e.clientY <= b.y + b.h) return this.stop();
      this.takeOver();
      this.mouse.x = e.clientX;
      this.mouse.y = e.clientY;
      if (e.button === 0) this.mouse.down = true;
      if (e.button === 2) this.throwGrenade(e.clientX, e.clientY);
    });
    on("pointerup", (e) => {
      if (e.button === 0) this.mouse.down = false;
    });
    on("contextmenu", (e) => e.preventDefault());
    on("wheel", (e) => this.pick((this.weapon + (e.deltaY > 0 ? 1 : WEAPONS.length - 1)) % WEAPONS.length), { passive: true });
  }

  private takeOver(): void {
    this.idle = 0;
    if (this.bot) {
      this.bot = null;
      this.say("Your turn!");
    }
  }

  private pick(i: number): void {
    if (i === this.weapon) return;
    this.weapon = i;
    this.cooldown = Math.min(this.cooldown, 0.15);
    this.sfx.thunk();
  }

  private say(text: string): void {
    this.toast = { text, life: 1.8 };
  }

  // ---------- simulation ----------

  private frame(now: number): void {
    if (!this.running) return;
    const dt = Math.min(1 / 30, (now - this.last) / 1000);
    this.last = now;
    this.update(dt);
    this.draw();
    requestAnimationFrame((t) => this.frame(t));
  }

  private update(dt: number): void {
    this.elapsed += dt;
    this.idle += dt;
    this.cooldown -= dt;
    this.grenadeCooldown -= dt;
    const p = this.player;

    let controls: Controls = {
      left: this.keys.has("a") || this.keys.has("arrowleft"),
      right: this.keys.has("d") || this.keys.has("arrowright"),
      jump: this.keys.has(" ") || this.keys.has("w") || this.keys.has("arrowup"),
      jumpPressed: this.jumpQueued,
    };
    this.jumpQueued = false;
    let aim = { x: this.mouse.x, y: this.mouse.y };
    let firing = this.mouse.down;

    if (this.bot) {
      const m = this.bot.think(dt, p, this.terrain);
      controls = m.controls;
      aim = { x: m.aimX, y: m.aimY };
      firing = m.fire;
      if (m.weapon !== null) this.pick(m.weapon);
      if (m.grenade) this.throwGrenade(m.aimX, m.aimY);
      if (this.elapsed > AUTOPLAY_SECONDS) {
        this.bot = null;
        this.say("Phew! Esc to put it all back.");
      }
    } else if (this.opts.autoplay && this.idle > IDLE_EXIT_SECONDS) {
      return this.stop();
    }
    this.aim = aim;

    p.update(dt, controls, this.terrain);
    if (Math.abs(aim.x - p.cx) > 4) p.facing = aim.x > p.cx ? 1 : -1;
    if (firing && this.cooldown <= 0) this.shoot(WEAPONS[this.weapon], aim.x, aim.y);

    this.updateProjectiles(dt, aim);
    this.updateParticles(dt);
    for (const f of this.flashes) f.life -= dt;
    this.flashes = this.flashes.filter((f) => f.life > 0);
    for (const b of this.beams) b.life -= dt;
    this.beams = this.beams.filter((b) => b.life > 0);
    this.shake = Math.max(0, this.shake - dt * 30);
    if (this.toast && (this.toast.life -= dt) <= 0) this.toast = null;

    const done = this.terrain.destroyed;
    while (this.milestone < MILESTONES.length && done >= MILESTONES[this.milestone][0]) {
      this.say(MILESTONES[this.milestone][1]);
      this.sfx.quack();
      this.milestone++;
    }
  }

  private aim = { x: 0, y: 0 };

  private muzzle(ax: number, ay: number): { x: number; y: number; angle: number } {
    const p = this.player;
    const cx = p.cx + p.facing * 6;
    const cy = p.y + p.h * 0.4;
    const angle = Math.atan2(ay - cy, ax - cx);
    return { x: cx + Math.cos(angle) * 22, y: cy + Math.sin(angle) * 22, angle };
  }

  private shoot(w: Weapon, ax: number, ay: number): void {
    this.cooldown = w.cooldown;
    this.lastShot = performance.now();
    const m = this.muzzle(ax, ay);
    if (w.shot === "laser") {
      const len = Math.hypot(this.w, this.h);
      const x1 = m.x + Math.cos(m.angle) * len;
      const y1 = m.y + Math.sin(m.angle) * len;
      this.beams.push({ x0: m.x, y0: m.y, x1, y1, life: 0.25 });
      const cells: [number, number][] = [];
      this.scorchLine(m.x, m.y, x1, y1, w.radius * 2.2);
      this.terrain.carveLine(m.x, m.y, x1, y1, w.radius, (x, y) => cells.push([x, y]));
      this.clearCells(cells, m.x, m.y, 180);
      this.sfx.zap();
      this.shake = Math.max(this.shake, 4);
      return;
    }
    this.projectiles.push(...fire(w, m.x, m.y, m.angle, this.rand));
    if (w.shot === "flame") {
      if (this.rand() < 0.25) this.sfx.fizz();
    } else if (w.shot === "missile") {
      this.sfx.quack();
    } else {
      this.sfx.pew();
    }
  }

  private throwGrenade(ax: number, ay: number): void {
    if (this.grenadeCooldown > 0) return;
    this.grenadeCooldown = GRENADE.cooldown;
    const m = this.muzzle(ax, ay);
    // Lob it: aim a little above the cursor.
    this.projectiles.push(...fire(GRENADE, m.x, m.y, m.angle - 0.25, this.rand));
    this.sfx.thunk();
  }

  private updateProjectiles(dt: number, aim: { x: number; y: number }): void {
    const keep: Projectile[] = [];
    for (const p of this.projectiles) {
      p.life -= dt;
      if (p.shot === "missile") steer(p, aim.x, aim.y, 5, dt);
      p.vy += p.gravity * dt;
      const nx = p.x + p.vx * dt;
      const ny = p.y + p.vy * dt;
      if (p.shot === "missile" && this.rand() < 0.6) this.puff(p.x, p.y, "#d8d8d8", 0.4);
      if (p.shot === "rocket" && this.rand() < 0.8) this.puff(p.x, p.y, "#f5f5f5", 0.5);

      if (p.shot === "grenade") {
        if (p.life <= 0) {
          this.explode(p.x, p.y, p.radius);
          continue;
        }
        // Bounce off whatever it meets.
        if (this.terrain.solidAt(nx, p.y)) p.vx *= -0.5;
        else p.x = nx;
        if (this.terrain.solidAt(p.x, ny)) {
          p.vy *= -0.45;
          p.vx *= 0.8;
        } else p.y = ny;
        keep.push(p);
        continue;
      }
      const hit = this.terrain.firstHit(p.x, p.y, nx, ny);
      if (hit) {
        if (p.shot === "rocket" || p.shot === "missile") this.explode(hit.x, hit.y, p.radius);
        else if (p.shot === "flame") this.burn(hit.x, hit.y, p.radius);
        else this.blast(hit.x, hit.y, p.radius, true);
        continue;
      }
      p.x = nx;
      p.y = ny;
      const out = p.x < -50 || p.x > this.w + 50 || p.y > this.h + 50 || p.y < -400;
      if (out || p.life <= 0) {
        if (p.life <= 0 && (p.shot === "rocket" || p.shot === "missile")) this.explode(p.x, p.y, p.radius * 0.7);
        continue;
      }
      keep.push(p);
    }
    this.projectiles = keep;
  }

  private updateParticles(dt: number): void {
    const keep: Particle[] = [];
    for (const q of this.particles) {
      q.life -= dt;
      if (q.life <= 0) continue;
      if (q.debris) {
        q.vy += 1500 * dt;
        const ny = q.y + q.vy * dt;
        // Settle on solid ground so rubble piles up on what's left.
        if (q.vy > 0 && (ny >= this.h - 1 || this.terrain.solidAt(q.x, ny))) {
          q.vy = 0;
          q.vx *= 0.5;
        } else q.y = ny;
        q.x += q.vx * dt;
      } else {
        q.vx *= 0.96;
        q.vy = q.vy * 0.96 - 30 * dt;
        q.x += q.vx * dt;
        q.y += q.vy * dt;
      }
      keep.push(q);
    }
    this.particles = keep.length > MAX_PARTICLES ? keep.slice(keep.length - MAX_PARTICLES) : keep;
  }

  // ---------- destruction ----------

  /** A small hit: punch a hole and spray what was there. */
  private blast(x: number, y: number, r: number, sound: boolean): void {
    this.scorch(x, y, r * 1.5, 0.35);
    const cells: [number, number][] = [];
    this.terrain.carveCircle(x, y, r, (cx, cy) => cells.push([cx, cy]));
    this.clearCells(cells, x, y, 260);
    if (sound && cells.length && this.rand() < 0.3) this.sfx.thunk();
  }

  private burn(x: number, y: number, r: number): void {
    this.scorch(x, y, r * 2.4, 0.18);
    if (this.rand() < 0.55) {
      const cells: [number, number][] = [];
      this.terrain.carveCircle(x, y, r, (cx, cy) => cells.push([cx, cy]));
      this.clearCells(cells, x, y, 80);
    }
    this.puff(x, y, this.rand() < 0.5 ? "#ffb347" : "#ff6a00", 0.3);
  }

  private explode(x: number, y: number, r: number): void {
    this.scorch(x, y, r * 1.6, 0.6);
    const cells: [number, number][] = [];
    this.terrain.carveCircle(x, y, r, (cx, cy) => cells.push([cx, cy]));
    this.clearCells(cells, x, y, 520);
    this.flashes.push({ x, y, r, life: 0.28, max: 0.28 });
    for (let i = 0; i < 14; i++) this.puff(x + (this.rand() - 0.5) * r, y + (this.rand() - 0.5) * r, i % 3 ? "#9a9a9a" : "#ffcf5a", 0.9);
    this.shake = Math.max(this.shake, r / 4);
    this.sfx.boom(r);
    const p = this.player;
    if (Math.hypot(p.cx - x, p.cy - y) < r * 2.2) p.knock(x, y, 420);
  }

  /** Erases blasted cells from the screen copy and throws some of them as debris. */
  private clearCells(cells: [number, number][], ox: number, oy: number, speed: number): void {
    if (!cells.length) return;
    const ctx = this.shotCtx;
    ctx.globalCompositeOperation = "source-over";
    const every = Math.max(1, Math.floor(cells.length / 220));
    cells.forEach(([x, y], i) => {
      ctx.clearRect(x - CELL / 2, y - CELL / 2, CELL, CELL);
      if (i % every !== 0 || this.particles.length >= MAX_PARTICLES) return;
      const px = Math.min(this.w - 1, Math.max(0, Math.floor(x)));
      const py = Math.min(this.h - 1, Math.max(0, Math.floor(y)));
      const o = (py * this.w + px) * 4;
      const a = Math.atan2(y - oy, x - ox) + (this.rand() - 0.5) * 0.8;
      const v = speed * (0.3 + this.rand() * 0.9);
      const life = 1.2 + this.rand() * 1.6;
      this.particles.push({
        x,
        y,
        vx: Math.cos(a) * v,
        vy: Math.sin(a) * v - speed * 0.35,
        life,
        max: life,
        size: CELL - (this.rand() < 0.5 ? 1 : 0),
        colour: `rgb(${this.pixels[o]},${this.pixels[o + 1]},${this.pixels[o + 2]})`,
        debris: true,
      });
    });
  }

  /** Darkens what's left of the screen around a hit (only where pixels remain). */
  private scorch(x: number, y: number, r: number, strength: number): void {
    const ctx = this.shotCtx;
    this.clipToGround(ctx);
    ctx.globalCompositeOperation = "source-atop";
    const g = ctx.createRadialGradient(x, y, r * 0.3, x, y, r);
    g.addColorStop(0, `rgba(30,18,8,${strength})`);
    g.addColorStop(1, "rgba(30,18,8,0)");
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  /** Scorch marks stay on the windows; the wallpaper around them isn't part of the level. */
  private clipToGround(ctx: CanvasRenderingContext2D): void {
    ctx.save();
    ctx.beginPath();
    for (const r of this.ground()) ctx.rect(r.x, r.y, r.w, r.h);
    ctx.clip();
  }

  private scorchLine(x0: number, y0: number, x1: number, y1: number, width: number): void {
    const ctx = this.shotCtx;
    this.clipToGround(ctx);
    ctx.globalCompositeOperation = "source-atop";
    ctx.strokeStyle = "rgba(120,0,30,0.45)";
    ctx.lineWidth = width;
    ctx.beginPath();
    ctx.moveTo(x0, y0);
    ctx.lineTo(x1, y1);
    ctx.stroke();
    ctx.restore();
  }

  private puff(x: number, y: number, colour: string, life: number): void {
    if (this.particles.length >= MAX_PARTICLES) return;
    this.particles.push({
      x,
      y,
      vx: (this.rand() - 0.5) * 60,
      vy: (this.rand() - 0.5) * 60,
      life,
      max: life,
      size: 3 + Math.floor(this.rand() * 4),
      colour,
      debris: false,
    });
  }

  // ---------- drawing ----------

  private draw(): void {
    const g = this.fx;
    const s = this.shake;
    this.root.style.transform = s > 0.5 ? `translate(${(this.rand() - 0.5) * s}px, ${(this.rand() - 0.5) * s}px)` : "";
    g.clearRect(0, 0, this.w, this.h);

    for (const q of this.particles) {
      g.globalAlpha = q.debris ? Math.min(1, q.life / 0.4) : Math.max(0, q.life / q.max) * 0.8;
      g.fillStyle = q.colour;
      g.fillRect(Math.round(q.x - q.size / 2), Math.round(q.y - q.size / 2), q.size, q.size);
    }
    g.globalAlpha = 1;

    for (const p of this.projectiles) this.drawProjectile(p);
    for (const b of this.beams) {
      const k = b.life / 0.25;
      g.strokeStyle = `rgba(255,59,107,${k})`;
      g.lineWidth = 10 * k + 2;
      g.beginPath();
      g.moveTo(b.x0, b.y0);
      g.lineTo(b.x1, b.y1);
      g.stroke();
      g.strokeStyle = `rgba(255,255,255,${k})`;
      g.lineWidth = 3 * k + 1;
      g.stroke();
    }
    for (const f of this.flashes) {
      const k = f.life / f.max;
      g.fillStyle = `rgba(255,240,170,${k * 0.9})`;
      g.beginPath();
      g.arc(f.x, f.y, f.r * (1.4 - k * 0.6), 0, Math.PI * 2);
      g.fill();
      g.strokeStyle = `rgba(255,140,30,${k})`;
      g.lineWidth = 4;
      g.stroke();
    }

    this.drawDuck();
    this.drawHud();
  }

  private drawProjectile(p: Projectile): void {
    const g = this.fx;
    g.fillStyle = p.colour;
    switch (p.shot) {
      case "bullet":
        g.fillRect(Math.round(p.x) - 2, Math.round(p.y) - 2, 4, 4);
        break;
      case "flame": {
        const k = Math.max(0, p.life / 0.45);
        g.fillStyle = k > 0.6 ? "#fff3a0" : k > 0.3 ? "#ff9f1c" : "#e2471b";
        const size = 6 + (1 - k) * 8;
        g.fillRect(Math.round(p.x - size / 2), Math.round(p.y - size / 2), size, size);
        break;
      }
      case "rocket":
      case "grenade":
        // An egg.
        g.fillStyle = "#2a1e14";
        g.fillRect(Math.round(p.x) - 5, Math.round(p.y) - 6, 10, 12);
        g.fillStyle = p.colour;
        g.fillRect(Math.round(p.x) - 4, Math.round(p.y) - 5, 8, 10);
        if (p.shot === "grenade" && Math.floor(p.life * 8) % 2 === 0) {
          g.fillStyle = "#e0453a";
          g.fillRect(Math.round(p.x) - 1, Math.round(p.y) - 9, 2, 3);
        }
        break;
      case "missile": {
        const a = Math.atan2(p.vy, p.vx);
        g.save();
        g.translate(p.x, p.y);
        g.rotate(a);
        g.fillStyle = "#2a1e14";
        g.fillRect(-9, -4, 16, 8);
        g.fillStyle = p.colour;
        g.fillRect(-8, -3, 12, 6);
        g.fillStyle = "#ff8c1a";
        g.fillRect(4, -2, 5, 4);
        g.restore();
        break;
      }
      case "laser":
        break;
    }
  }

  private drawDuck(): void {
    const p = this.player;
    const now = performance.now();
    let frame: FrameName;
    if (now - this.lastShot < 90) frame = "peck";
    else if (!p.onGround) frame = p.flapping ? (Math.floor(now / 70) % 2 ? "flap_up" : "flap_down") : "flap_up";
    else if (Math.abs(p.vx) > 20) frame = Math.floor(p.walked / 14) % 2 ? "walk1" : "walk2";
    else frame = Math.floor(now / 2500) % 8 === 0 ? "blink" : "idle";
    this.sprite.draw(frame, p.x, p.y, p.facing < 0);

    // The blaster, pointing where the duck aims.
    const m = this.muzzle(this.aim.x, this.aim.y);
    const g = this.fx;
    g.save();
    g.translate(p.cx + p.facing * 6, p.y + p.h * 0.4);
    g.rotate(m.angle);
    g.fillStyle = "#2a1e14";
    g.fillRect(6, -4, 18, 8);
    g.fillStyle = WEAPONS[this.weapon].colour;
    g.fillRect(8, -2, 14, 4);
    g.restore();

    if (p.fuel < 0.99) {
      g.fillStyle = "rgba(42,30,20,0.7)";
      g.fillRect(p.x, p.y - 10, p.w, 5);
      g.fillStyle = p.fuel > 0.25 ? "#7ed957" : "#e0453a";
      g.fillRect(p.x + 1, p.y - 9, (p.w - 2) * p.fuel, 3);
    }
  }

  private drawHud(): void {
    const g = this.fx;
    const n = WEAPONS.length;
    const box = 38;
    const gap = 6;
    const barW = n * box + (n - 1) * gap;
    const x0 = Math.round((this.w - barW) / 2);
    const y0 = 14;

    g.fillStyle = "rgba(255,253,246,0.92)";
    g.strokeStyle = "#2a1e14";
    g.lineWidth = 2;
    g.beginPath();
    g.roundRect(x0 - 12, y0 - 8, barW + 24, 100, 10);
    g.fill();
    g.stroke();

    const pct = Math.round(this.terrain.destroyed * 100);
    g.fillStyle = "#2a1e14";
    g.font = "bold 13px 'Segoe UI', system-ui, sans-serif";
    g.textAlign = "left";
    g.fillText(`DESTROYED ${pct}%`, x0, y0 + 10);
    g.fillStyle = "#e7dccb";
    g.fillRect(x0 + 120, y0 + 1, barW - 120, 10);
    g.fillStyle = "#e0453a";
    g.fillRect(x0 + 120, y0 + 1, ((barW - 120) * pct) / 100, 10);

    WEAPONS.forEach((w, i) => {
      const x = x0 + i * (box + gap);
      const y = y0 + 20;
      const on = i === this.weapon;
      g.fillStyle = on ? "#ffd23f" : "#fff";
      g.fillRect(x, y, box, box);
      g.strokeStyle = "#2a1e14";
      g.lineWidth = on ? 3 : 1.5;
      g.strokeRect(x, y, box, box);
      g.fillStyle = "#2a1e14";
      g.font = "bold 10px monospace";
      g.textAlign = "left";
      g.fillText(String(i + 1), x + 3, y + 11);
      g.font = "20px 'Segoe UI Symbol', 'Noto Sans Symbols 2', sans-serif";
      g.textAlign = "center";
      g.fillStyle = w.colour === "#ffffff" || w.colour === "#fff6dc" ? "#8a7a68" : w.colour;
      g.fillText(w.icon, x + box / 2, y + box / 2 + 9);
    });
    g.fillStyle = "#2a1e14";
    g.font = "bold 12px 'Segoe UI', system-ui, sans-serif";
    g.textAlign = "center";
    g.fillText(WEAPONS[this.weapon].name + "  ·  right-click: egg grenade", this.w / 2, y0 + 78);

    // Exit button, always clickable even if the keyboard isn't ours.
    const ex = { x: x0 + barW + 22, y: y0 - 8, w: 34, h: 34 };
    this.exitBox = ex;
    g.fillStyle = "#fffdf6";
    g.fillRect(ex.x, ex.y, ex.w, ex.h);
    g.strokeStyle = "#2a1e14";
    g.lineWidth = 2;
    g.strokeRect(ex.x, ex.y, ex.w, ex.h);
    g.fillStyle = "#2a1e14";
    g.font = "bold 16px sans-serif";
    g.fillText("✕", ex.x + ex.w / 2, ex.y + 23);

    const hint = this.bot
      ? "Waddle is playing! Press any key or click to take over · Esc to quit"
      : this.elapsed < 10
        ? "A/D move · Space jump, hold to flap · mouse aim & shoot · 1-7 / wheel weapons · R repair · M mute · Esc quit"
        : "";
    if (hint) {
      g.font = "bold 14px 'Segoe UI', system-ui, sans-serif";
      const tw = g.measureText(hint).width + 28;
      g.fillStyle = "rgba(42,30,20,0.82)";
      g.beginPath();
      g.roundRect((this.w - tw) / 2, this.h - 48, tw, 30, 15);
      g.fill();
      g.fillStyle = "#fffdf6";
      g.fillText(hint, this.w / 2, this.h - 28);
    }
    if (this.toast) {
      const k = Math.min(1, this.toast.life / 0.4);
      g.globalAlpha = k;
      g.font = "bold 40px 'Segoe UI', system-ui, sans-serif";
      g.lineWidth = 6;
      g.strokeStyle = "#2a1e14";
      g.strokeText(this.toast.text, this.w / 2, this.h * 0.32);
      g.fillStyle = "#ffd23f";
      g.fillText(this.toast.text, this.w / 2, this.h * 0.32);
      g.globalAlpha = 1;
    }
  }
}
