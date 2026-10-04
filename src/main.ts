// Overlay entry point: the render/behaviour loop and the wiring between the
// duck, its UI, and the backend's events.

import { Duck, pickWanderTarget } from "./body/behavior";
import { alarmPalette, buildPalette } from "./body/palette";
import { standBeside } from "./body/pathfind";
import { computeSegments, type Segment, type WinRect } from "./body/platforms";
import { SpriteRenderer } from "./body/renderer";
import { api, on, type Events, type HitRect, type Platform } from "./ipc";
import { Game } from "./play/game";
import { ApprovalCard } from "./ui/approval";
import { Bubble } from "./ui/bubble";
import { Chat } from "./ui/chat";

const SCALE = 4; // sprite pixels → logical pixels (16x14 → 64x56)
const IDLE_TICK_MS = 120;
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
/** Play mode owns the screen while set; the normal duck and its UI step aside. */
let game: Game | null = null;
let playStarting = false;

const duck = new Duck({ w: renderer.width, h: renderer.height }, screen.w * 0.7, screen.h * 0.4);

const bubble = new Bubble(document.getElementById("bubble")!, () => void api.halt());
const approval = new ApprovalCard(document.getElementById("approval")!, (id, ok) => void api.answerApproval(id, ok));
const chat = new Chat(document.getElementById("chat") as HTMLFormElement, {
  send: (text) => {
    bubble.say("user", text);
    void api.sendMessage(text);
  },
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
  if (game || playStarting) {
    last = now;
    schedule(now);
    return;
  }
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
  else window.setTimeout(() => requestAnimationFrame(loop), IDLE_TICK_MS);
}

function think(now: number): void {
  if (duck.mode !== "idle" || busy || approval.visible || chat.visible) {
    nextWander = Math.max(nextWander, now + 3000);
    return;
  }
  if (now - lastInteraction > SLEEP_AFTER_MS) {
    duck.sleeping = true;
    return;
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
  if (duck.sleeping) {
    ctx.fillStyle = "#2a1e14";
    ctx.font = "bold 14px monospace";
    ctx.fillText("z", r.x + r.w - 6, r.y + 6 - (Math.floor(now / 600) % 3) * 4);
  }
  lastRect = { ...r, y: r.y - 16, h: r.h + 16 };
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
  for (const el of [bubble.el, chat.el, approval.el]) {
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

// ---------- play mode ----------

const nextFrame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));

async function startPlay(req: Events["play:start"]): Promise<void> {
  if (game || playStarting) return;
  if (approval.visible) {
    bubble.say("notice", "Let's finish this first: answer the card, then we can play.");
    return;
  }
  playStarting = true;
  touch();
  chat.close();
  // Step out of the picture so the copy shows only the user's windows.
  for (const el of [bubble.el, chat.el]) el.style.visibility = "hidden";
  ctx.clearRect(0, 0, screen.w, screen.h);
  await nextFrame();
  await nextFrame();
  await new Promise((r) => window.setTimeout(r, 80));
  try {
    const image = await api.playSnapshot();
    game = new Game({
      image,
      windows: windows.map(({ x, y, w, h }) => ({ x, y, w, h })),
      duck: { x: duck.body.x, y: duck.body.y },
      palette: buildPalette(baseColor),
      autoplay: req.autoplay,
      target: req.target ?? null,
      weapon: req.weapon ?? null,
      onExit: endPlay,
    });
    await api.playInput(true);
    await game.start();
  } catch (e) {
    game = null;
    void api.playInput(false);
    bubble.say("error", `Couldn't start play mode: ${e}`);
  } finally {
    playStarting = false;
    for (const el of [bubble.el, chat.el]) el.style.visibility = "";
  }
}

function endPlay(feet: { x: number; y: number }, destroyed: number): void {
  game = null;
  void api.playInput(false);
  // Drop back into the real desktop where the game left off.
  duck.body.x = feet.x;
  duck.body.y = feet.y;
  duck.body.vy = 0;
  duck.body.grounded = false;
  duck.mode = "falling";
  ctx.clearRect(0, 0, screen.w, screen.h);
  lastRect = null;
  sentRects = "";
  touch();
  const pct = Math.round(destroyed * 100);
  bubble.say("planner", pct > 0 ? `Phew! ${pct}% wrecked, and it's all back: that was only a copy.` : "Okay, back to normal!");
}

void on("play:start", (req) => void startPlay(req));
void on("play:stop", () => game?.stop());

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

void on("busy", (b) => {
  busy = b;
  bubble.setBusy(b);
  if (!b) {
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
      // The planner's final words already streamed; show the message only if it didn't.
      if (ev.outcome !== "done") bubble.say(ev.outcome === "failed" ? "error" : "notice", ev.message);
      break;
    case "notice":
      bubble.say("notice", ev.text);
      break;
    case "trace_saved":
      bubble.rate((good) => void api.rateTask(ev.task_id, good));
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

void on("chat:open", ({ voice }) => {
  touch();
  chat.open();
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
  } catch (e) {
    bubble.say("error", `Couldn't reach the backend: ${e}`);
  }
  applyPalette();
  requestAnimationFrame(loop);
}

void start();
