// Overlay entry point: the render/behaviour loop and the wiring between the
// duck, its UI, and the backend's events.

import { Duck, pickWanderTarget, usesLaptop } from "./body/behavior";
import { alarmPalette, buildPalette } from "./body/palette";
import { planIntent, type Plan } from "./body/intent";
import { standBeside } from "./body/pathfind";
import { computeSegments, type Segment, type WinRect } from "./body/platforms";
import { SpriteRenderer } from "./body/renderer";
import { api, on, type HitRect, type NudgePayload, type Platform } from "./ipc";
import { ApprovalCard } from "./ui/approval";
import { Bubble } from "./ui/bubble";
import { Chat } from "./ui/chat";
import { chime, Pointer } from "./ui/pointer";

const SCALE = 4; // sprite pixels → logical pixels (16x14 → 64x56)
const IDLE_TICK_MS = 120;
/** Idle frames are further apart on battery. */
let idleTick = IDLE_TICK_MS;
const SLEEP_AFTER_MS = 120_000;

const canvas = document.getElementById("stage") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;
const renderer = new SpriteRenderer(ctx, SCALE);
const screen = { w: window.innerWidth, h: window.innerHeight };

let windows: WinRect[] = [];
let segments: Segment[] = [];
const moved = new Map<number, [number, number]>();
let baseColor = "#FFD23F";
let wander = true;
let busy = false;
let alarm = false;
let nextWander = performance.now() + 4000;
let lastInteraction = performance.now();
let lastRect: HitRect | null = null;
let sentRects = "";
let hoverTimer = 0;
/** What the ambient decision asked for (perch, explore…), worked through stop by stop. */
let plan: Plan | null = null;
let planStop = 0;

const duck = new Duck({ w: renderer.width, h: renderer.height }, screen.w * 0.7, screen.h * 0.4);

const pointer = new Pointer();
const bubble = new Bubble(document.getElementById("bubble")!, () => void api.halt());
const approval = new ApprovalCard(
  document.getElementById("approval")!,
  (id, ok, draft) => void api.answerApproval(id, ok, draft ?? null),
  (id) => void api.undoSend(id),
);
const chat = new Chat(document.getElementById("chat") as HTMLFormElement, {
  send: (text, selection) => {
    bubble.say("user", selection ? `${text || "Help with this"} 📎` : text);
    void api.sendMessage(text, selection);
  },
  dropSelection: () => void api.dropSelection(),
  // A local model loads while the user types.
  opened: () => void api.warmUp(),
  voiceStart: () => api.voiceStart(),
  voiceStop: () => api.voiceStop(),
  error: (m) => bubble.say("error", m),
});

function resize(): void {
  const dpr = window.devicePixelRatio || 1;
  screen.w = window.innerWidth;
  screen.h = window.innerHeight;
  canvas.width = Math.round(screen.w * dpr);
  canvas.height = Math.round(screen.h * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.imageSmoothingEnabled = false;
  lastRect = null;
  rebuildSegments();
  duck.keepOnScreen(screen);
}

function rebuildSegments(): void {
  segments = computeSegments(windows, { width: screen.w, floorY: screen.h, minHeadroom: renderer.height, minWidth: 24 });
}

function setWindows(list: Platform[]): void {
  const prev = new Map(windows.map((w) => [w.id, w]));
  for (const w of list) {
    const p = prev.get(w.id);
    if (p && (p.x !== w.x || p.y !== w.y)) moved.set(w.id, [w.x - p.x, w.y - p.y]);
  }
  windows = list;
  rebuildSegments();
}

function applyPalette(): void {
  renderer.setPalette(alarm ? alarmPalette() : buildPalette(baseColor));
  lastRect = null;
}

function touch(): void {
  lastInteraction = performance.now();
  duck.sleeping = false;
}

// ---------- render & behaviour loop ----------

let last = performance.now();
let scheduled = false;

function loop(now: number): void {
  scheduled = false;
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  duck.update(dt, segments, moved);
  moved.clear();
  think(now);
  draw(now);
  layoutUi();
  schedule(now);
}

function schedule(now: number): void {
  if (scheduled) return;
  scheduled = true;
  // Full frame rate while anything moves; a slow tick when idle keeps CPU near zero.
  if (duck.isAnimating(now) || bubble.visible || approval.visible) requestAnimationFrame(loop);
  else window.setTimeout(() => requestAnimationFrame(loop), idleTick);
}

function think(now: number): void {
  // Stay put while the bubble is up: its buttons are hard to hit on a moving duck.
  if (duck.mode !== "idle" || busy || approval.visible || chat.visible || bubble.visible) {
    nextWander = Math.max(nextWander, now + 3000);
    return;
  }
  if (now - lastInteraction > SLEEP_AFTER_MS) {
    duck.sleeping = true;
    return;
  }
  if (wander && now > nextWander && plan) {
    if (planStop < plan.stops.length) {
      const p = plan;
      const stop = p.stops[planStop++];
      const last = planStop === p.stops.length;
      const facing = last && p.faceX !== undefined ? (p.faceX > stop.x ? 1 : -1) : undefined;
      nextWander = now + p.pauseMs;
      duck.moveTo(stop, segments, {
        facing,
        onArrive: () => {
          if (p.inspect) duck.doAct("look", performance.now(), 700);
        },
      });
      return;
    }
    plan = null;
  }
  if (wander && now > nextWander) {
    nextWander = now + 5000 + Math.random() * 9000;
    const t = pickWanderTarget(segments, renderer.width);
    if (t && Math.abs(t.x - duck.body.x) > 40) duck.moveTo(t, segments);
  }
}

function draw(now: number): void {
  const r = duck.rect;
  const frame = duck.frame(now);
  if (lastRect) ctx.clearRect(lastRect.x - 2, lastRect.y - 2, lastRect.w + 4, lastRect.h + 4);
  renderer.draw(frame, r.x, r.y, duck.facing < 0);
  if (duck.laptop && duck.mode === "idle") drawLaptop(r);
  if (duck.sleeping) {
    ctx.fillStyle = "#2a1e14";
    ctx.font = "bold 14px monospace";
    ctx.fillText("z", r.x + r.w - 6, r.y + 6 - (Math.floor(now / 600) % 3) * 4);
  }
  // The laptop sits just outside the sprite, so clear a little wider.
  const pad = renderer.scale * LAPTOP[0].length;
  lastRect = { x: r.x - pad, y: r.y - 16, w: r.w + pad * 2, h: r.h + 16 };
}

// A tiny open laptop, 7x5 sprite pixels: lid (L), glowing screen (S), base (B).
const LAPTOP = [".LLLLL.", ".LSSSL.", ".LSSSL.", ".LLLLL.", "BBBBBBB"];
const LAPTOP_COLOURS: Record<string, string> = { L: "#3a3f4b", S: "#9ad1ff", B: "#5b6170" };

function drawLaptop(r: HitRect): void {
  const px = renderer.scale;
  const w = LAPTOP[0].length * px;
  const x = duck.facing > 0 ? r.x + r.w - px : r.x - w + px;
  const y = r.y + r.h - LAPTOP.length * px;
  // The screen flickers as the duck types.
  const glow = Math.floor(performance.now() / 220) % 3 === 0 ? "#c4e6ff" : LAPTOP_COLOURS.S;
  LAPTOP.forEach((row, j) =>
    [...row].forEach((k, i) => {
      if (k === ".") return;
      ctx.fillStyle = k === "S" ? glow : LAPTOP_COLOURS[k];
      ctx.fillRect(Math.round(x + i * px), Math.round(y + j * px), px, px);
    }),
  );
}

function union(a: HitRect, b: DOMRect): HitRect {
  const x = Math.min(a.x, b.left);
  const y = Math.min(a.y, b.top);
  return { x, y, w: Math.max(a.x + a.w, b.right) - x, h: Math.max(a.y + a.h, b.bottom) - y };
}

function layoutUi(): void {
  const r = duck.rect;
  chat.place(r, screen);
  const chatBox = chat.visible ? chat.el.getBoundingClientRect() : null;
  // When the chat box sits above the duck, the bubble goes above both.
  bubble.place(chatBox && chatBox.top < r.y ? union(r, chatBox) : r, screen);
  const avoid = [bubble.visible ? bubble.el.getBoundingClientRect() : null, chatBox].filter((b): b is DOMRect => b !== null);
  approval.place(r, screen, avoid);
  // Tell the backend which areas should catch the mouse.
  const rects: HitRect[] = [r];
  if (!badge.classList.contains("hidden")) {
    badge.style.transform = `translate(${Math.round(r.x + r.w - 6)}px, ${Math.round(r.y - 22)}px)`;
  }
  for (const el of [bubble.el, chat.el, approval.el, badge]) {
    if (el.classList.contains("hidden")) continue;
    const b = el.getBoundingClientRect();
    rects.push({ x: b.left, y: b.top, w: b.width, h: b.height });
  }
  const key = JSON.stringify(rects.map((x) => [Math.round(x.x), Math.round(x.y), Math.round(x.w), Math.round(x.h)]));
  if (key !== sentRects) {
    sentRects = key;
    void api.setHitRects(rects);
  }
}

// ---------- pointer: click to chat, drag to carry, right-click for settings ----------

let press: { x: number; y: number; dragging: boolean } | null = null;

function overDuck(x: number, y: number): boolean {
  const r = duck.rect;
  return x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h;
}

canvas.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || !overDuck(e.clientX, e.clientY)) return;
  press = { x: e.clientX, y: e.clientY, dragging: false };
  canvas.setPointerCapture(e.pointerId);
  touch();
});

canvas.addEventListener("pointermove", (e) => {
  if (!press) return;
  if (!press.dragging && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 5) {
    press.dragging = true;
    duck.grab();
    void api.setCapture(true);
  }
  if (press.dragging) duck.dragTo(e.clientX, e.clientY + duck.size.h / 2);
});

canvas.addEventListener("pointerup", (e) => {
  if (!press) return;
  canvas.releasePointerCapture(e.pointerId);
  if (press.dragging) {
    duck.release();
    void api.setCapture(false);
  } else {
    chat.toggle();
  }
  press = null;
});

canvas.addEventListener("dblclick", (e) => {
  if (busy && overDuck(e.clientX, e.clientY)) void api.halt();
});

canvas.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (overDuck(e.clientX, e.clientY)) void api.openSettings();
});

// ---------- backend events ----------

void on("desktop:windows", setWindows);
void on("power:battery", (b) => (idleTick = b ? 250 : IDLE_TICK_MS));
void on("monitor:moved", ({ from }) => {
  // The window may not have its new size yet; enter once it has.
  window.setTimeout(() => {
    duck.enterFrom(from, screen);
    touch();
  }, 150);
});

void on("duck:move", ({ id, x, y, purpose }) => {
  touch();
  window.clearTimeout(hoverTimer);
  const spot = standBeside({ x, y }, duck.size, screen.w);
  if (Math.hypot(duck.body.x - spot.x, duck.body.y - spot.y) < 10) {
    duck.facing = spot.facing;
    void api.duckArrived(id);
    return;
  }
  duck.moveTo(spot, segments, {
    maxSeconds: purpose === "approach" ? 2.5 : 1.2,
    facing: spot.facing,
    onArrive: () => void api.duckArrived(id),
  });
});

void on("duck:act", ({ kind }) => {
  touch();
  duck.doAct(kind, performance.now(), kind === "type" ? 900 : 600);
});

void on("duck:point", ({ x, y, label }) => {
  touch();
  pointer.show(x, y, label, screen);
  duck.doAct("look", performance.now(), 800);
});

// Nudges: a "!" badge while the user is in full screen, the full nudge in the bubble otherwise.
const badge = document.getElementById("badge") as HTMLButtonElement;
const waiting = new Map<string, NudgePayload>();
const shownFull = new Set<string>();

function showNudge(n: NudgePayload): void {
  waiting.delete(n.id);
  badge.classList.toggle("hidden", waiting.size === 0);
  if (shownFull.has(n.id)) return;
  shownFull.add(n.id);
  duck.doAct("peck", performance.now(), 900);
  bubble.nudge(n.text, n.actions, (action) => api.nudgeAction(n.id, action));
}

void on("nudge", (n) => {
  if (n.stage === "full") {
    showNudge(n);
    return;
  }
  if (shownFull.has(n.id)) return;
  waiting.set(n.id, n);
  badge.classList.remove("hidden");
  badge.title = n.text;
});

void on("nudge:chime", () => chime());

badge.addEventListener("click", () => {
  touch();
  for (const n of [...waiting.values()]) showNudge({ ...n, stage: "full" });
});

void on("reminder", ({ text, late }) => {
  touch();
  chime();
  duck.doAct("peck", performance.now(), 900);
  bubble.say("reminder", `⏰ Reminder${late ? " (from while I was off)" : ""}: ${text}`);
});

void on("duck:intent", ({ intent, window: win, cursor }) => {
  if (busy || !wander) return;
  const now = performance.now();
  if (intent === "nap") {
    plan = null;
    duck.sleeping = true;
    nextWander = now + 600_000;
    return;
  }
  // A fresh decision counts as a reason to be up and about.
  lastInteraction = now;
  plan = planIntent(intent, win, segments, renderer.width, cursor, duck.body.x);
  planStop = 0;
  nextWander = now;
});

void on("busy", (b) => {
  busy = b;
  plan = null;
  bubble.setBusy(b);
  if (!b) {
    duck.laptop = false;
    alarm = false;
    applyPalette();
    // Flutter back down after a task that ended mid-air.
    hoverTimer = window.setTimeout(() => duck.land(segments), 1500);
  }
});

void on("approval", (req) => {
  touch();
  approval.show(req);
  alarm = req.countdown_ms === null;
  applyPalette();
});

void on("undo", ({ id, secs }) => {
  touch();
  approval.showUndo(id, secs);
});

void on("undo:done", ({ id, undone }) => {
  approval.hideUndo(id);
  if (undone) bubble.say("notice", "Not sent.");
});

void on("agent", (ev) => {
  touch();
  switch (ev.type) {
    case "text_delta":
      bubble.stream(ev.lane, ev.text);
      break;
    case "text_done":
      bubble.endStream(ev.lane);
      break;
    case "tool_started":
      bubble.tool(ev.summary);
      // Email, calendar and file reads: the duck pecks at its little laptop until it acts on screen again.
      duck.laptop = usesLaptop(ev.tool);
      if (ev.tool === "run_command" || ev.tool === "write_file") duck.doAct("type", performance.now(), 1200);
      break;
    case "tool_output":
      bubble.output(ev.line);
      break;
    case "tool_finished":
      if (!ev.ok) bubble.say("error", ev.summary);
      break;
    case "approval_resolved":
      approval.resolve(ev.id);
      alarm = false;
      applyPalette();
      break;
    case "task_finished":
      duck.laptop = false;
      // The planner's final words already streamed; show the message only if it didn't.
      if (ev.outcome !== "done") bubble.say(ev.outcome === "failed" ? "error" : "notice", ev.message);
      break;
    case "notice":
      bubble.say("notice", ev.text);
      break;
    case "trace_saved":
      bubble.rate((good) => void api.rateTask(ev.task_id, good));
      break;
    case "offer":
      bubble.offer(ev.label, () => api.openAnswer(ev.id));
      break;
    case "task_started":
    case "thinking":
      break;
  }
});

void on("settings", (s) => {
  baseColor = s.color;
  wander = s.wander;
  applyPalette();
});

void on("chat:open", ({ voice, selection }) => {
  touch();
  chat.open();
  if (selection !== undefined) {
    chat.setSelection(selection);
    if (!selection) bubble.say("notice", "I couldn't find any selected text. Select some, then press Ctrl+Alt+A.");
  }
  if (voice) void chat.toggleVoice();
});

void on("wander:toggle", () => {
  wander = !wander;
  bubble.say("notice", wander ? "Back to exploring!" : "I'll stay put.");
});

window.addEventListener("resize", resize);

async function start(): Promise<void> {
  resize();
  try {
    const boot = await api.bootstrap();
    baseColor = boot.color;
    wander = boot.wander;
    setWindows(boot.windows);
    if (boot.demo) {
      bubble.say(
        "planner",
        "Hi, I'm Waddle! I'm in demo mode until you add an API key. Click me and ask for anything, or right-click me for Settings.",
      );
    } else {
      bubble.say("planner", "Hi! Click me (or press Ctrl+Alt+Space) and tell me what to do.");
    }
    for (const n of boot.notices) bubble.say("error", n);
  } catch (e) {
    bubble.say("error", `Couldn't reach the backend: ${e}`);
  }
  applyPalette();
  requestAnimationFrame(loop);
}

void start();
