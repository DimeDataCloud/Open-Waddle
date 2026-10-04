// Play-mode weapons: duck-themed versions of the classic arcade arsenal.
// Numbers 1-7 pick one; right-click always throws an egg grenade.

export type Shot = "bullet" | "rocket" | "laser" | "flame" | "missile" | "grenade";

export interface Weapon {
  id: string;
  name: string;
  /** Short label for the weapon bar. */
  icon: string;
  shot: Shot;
  /** Seconds between shots while the button is held. */
  cooldown: number;
  speed: number;
  /** Blast radius in px. */
  radius: number;
  /** Random spread per projectile, radians. */
  spread: number;
  count: number;
  colour: string;
}

export const WEAPONS: Weapon[] = [
  { id: "pea", name: "Pea shooter", icon: "•", shot: "bullet", cooldown: 0.2, speed: 1300, radius: 9, spread: 0.01, count: 1, colour: "#7ed957" },
  { id: "crumbs", name: "Crumb blaster", icon: "⁘", shot: "bullet", cooldown: 0.055, speed: 1400, radius: 6, spread: 0.08, count: 1, colour: "#e8c07a" },
  { id: "feathers", name: "Feather shotgun", icon: "⋔", shot: "bullet", cooldown: 0.55, speed: 1150, radius: 7, spread: 0.3, count: 8, colour: "#ffffff" },
  { id: "egg", name: "Egg bazooka", icon: "◖", shot: "rocket", cooldown: 0.75, speed: 720, radius: 48, spread: 0, count: 1, colour: "#fff6dc" },
  { id: "laser", name: "Laser eyes", icon: "⌁", shot: "laser", cooldown: 0.8, speed: 0, radius: 6, spread: 0, count: 1, colour: "#ff3b6b" },
  { id: "flame", name: "Hot sauce", icon: "♨", shot: "flame", cooldown: 0.018, speed: 520, radius: 5, spread: 0.22, count: 1, colour: "#ff8a1f" },
  { id: "quack", name: "Quack missiles", icon: "➶", shot: "missile", cooldown: 0.5, speed: 540, radius: 34, spread: 0.5, count: 2, colour: "#ffd23f" },
];

export const GRENADE: Weapon = {
  id: "grenade",
  name: "Egg grenade",
  icon: "◍",
  shot: "grenade",
  cooldown: 0.6,
  speed: 760,
  radius: 64,
  spread: 0,
  count: 1,
  colour: "#fff6dc",
};

export function weaponIndex(id: string | null | undefined): number {
  const i = WEAPONS.findIndex((w) => w.id === id);
  return i < 0 ? 0 : i;
}

export interface Projectile {
  shot: Shot;
  x: number;
  y: number;
  vx: number;
  vy: number;
  /** Seconds left before it fizzles (or, for grenades, explodes). */
  life: number;
  radius: number;
  colour: string;
  gravity: number;
}

/** Spawns the projectiles for one trigger pull from (x, y) toward `angle`. */
export function fire(w: Weapon, x: number, y: number, angle: number, rand: () => number): Projectile[] {
  const out: Projectile[] = [];
  for (let i = 0; i < w.count; i++) {
    const a = angle + (rand() - 0.5) * 2 * w.spread;
    const speed = w.speed * (w.shot === "flame" ? 0.7 + rand() * 0.6 : 1);
    out.push({
      shot: w.shot,
      x,
      y,
      vx: Math.cos(a) * speed,
      vy: Math.sin(a) * speed,
      life: { bullet: 1.6, rocket: 3, laser: 0, flame: 0.32 + rand() * 0.15, missile: 4, grenade: 1.7 }[w.shot],
      radius: w.radius,
      colour: w.colour,
      gravity: { bullet: 60, rocket: 160, laser: 0, flame: -120, missile: 0, grenade: 1400 }[w.shot],
    });
  }
  return out;
}

/** Turns a missile's velocity toward a target point, at most `turn` radians per second. */
export function steer(p: Projectile, tx: number, ty: number, turn: number, dt: number): void {
  const speed = Math.hypot(p.vx, p.vy);
  const cur = Math.atan2(p.vy, p.vx);
  let diff = Math.atan2(ty - p.y, tx - p.x) - cur;
  while (diff > Math.PI) diff -= 2 * Math.PI;
  while (diff < -Math.PI) diff += 2 * Math.PI;
  const a = cur + Math.max(-turn * dt, Math.min(turn * dt, diff));
  p.vx = Math.cos(a) * speed;
  p.vy = Math.sin(a) * speed;
}
