// Tricks the duck does when asked ("fly around the screen", "come here",
// "make a mess"): a list of steps worked out from the screen size and where the
// duck and the pointer are. Pure, so it's tested without a screen.

export type Trick = "fly_around" | "come_here" | "dance" | "nap" | "wake_up" | "hide" | "mess";

export type TrickStep =
  | { kind: "fly"; x: number; y: number }
  | { kind: "act"; act: "peck" | "look" | "type"; ms: number; fx?: "dust" | "sparkle" | "heart" }
  | { kind: "face"; dir: 1 | -1 }
  | { kind: "sleep" }
  | { kind: "wake" };

export const TRICKS: readonly Trick[] = ["fly_around", "come_here", "dance", "nap", "wake_up", "hide", "mess"];

/** Standing spots (feet) the duck may fly to: inside the screen with room for its body. */
function clamp(p: { x: number; y: number }, screen: { w: number; h: number }, size: { w: number; h: number }): { x: number; y: number } {
  const m = 8;
  return {
    x: Math.round(Math.min(screen.w - size.w / 2 - m, Math.max(size.w / 2 + m, p.x))),
    y: Math.round(Math.min(screen.h - m, Math.max(size.h + m, p.y))),
  };
}

export function planTrick(
  trick: Trick,
  screen: { w: number; h: number },
  size: { w: number; h: number },
  at: { x: number; y: number },
  cursor: { x: number; y: number } | null,
  rand: () => number = Math.random,
): TrickStep[] {
  const fly = (x: number, y: number): TrickStep => ({ kind: "fly", ...clamp({ x, y }, screen, size) });
  switch (trick) {
    case "fly_around": {
      // A loop round the screen, high then low, ending where it started.
      const loop: [number, number][] = [
        [0.15, 0.3],
        [0.5, 0.15],
        [0.85, 0.3],
        [0.8, 0.6],
        [0.5, 0.5],
        [0.2, 0.6],
      ];
      return [...loop.map(([fx, fy]) => fly(fx * screen.w, fy * screen.h)), fly(at.x, at.y), { kind: "act", act: "look", ms: 600 }];
    }
    case "come_here": {
      const target = cursor ?? { x: screen.w / 2, y: screen.h * 0.6 };
      // Beside the pointer, not under it, facing it.
      const side = target.x > screen.w / 2 ? -1 : 1;
      return [fly(target.x + side * (size.w * 0.8), target.y + size.h * 0.4), { kind: "face", dir: side > 0 ? -1 : 1 }, { kind: "act", act: "look", ms: 700 }];
    }
    case "dance": {
      const hop = (dir: 1 | -1): TrickStep[] => [{ kind: "face", dir }, fly(at.x, at.y - 36), fly(at.x, at.y)];
      return [...hop(1), ...hop(-1), { kind: "act", act: "peck", ms: 350 }, ...hop(1), { kind: "act", act: "look", ms: 500, fx: "heart" }];
    }
    case "nap":
      return [{ kind: "sleep" }];
    case "wake_up":
      return [{ kind: "wake" }, { kind: "act", act: "look", ms: 600 }];
    case "hide": {
      const right = at.x > screen.w / 2;
      return [fly(right ? screen.w : 0, screen.h), { kind: "face", dir: right ? 1 : -1 }];
    }
    case "mess": {
      // Pecks all over the screen in a puff of dust; nothing on it is touched.
      const steps: TrickStep[] = [];
      for (let i = 0; i < 6; i++) {
        steps.push(fly((0.1 + rand() * 0.8) * screen.w, (0.25 + rand() * 0.65) * screen.h));
        steps.push({ kind: "act", act: "peck", ms: 300, fx: "dust" });
      }
      steps.push(fly(at.x, at.y), { kind: "act", act: "look", ms: 600, fx: "sparkle" });
      return steps;
    }
  }
}
