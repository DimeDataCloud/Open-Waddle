// The speech bubble: a tiny live transcript above the duck. Planner narration
// and quick replies stream into their own lines; tool activity shows as a
// status line with the latest output.

export type LineKind = "planner" | "quick" | "user" | "tool" | "output" | "notice" | "error" | "reminder";

const MAX_LINES = 5;

/** Models sometimes answer in Markdown; the bubble is plain text, so drop the markup. */
export function plain(text: string): string {
  return text
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .replace(/__(.+?)__/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\[([^\]]+)\]\((https?:[^)]+)\)/g, "$1");
}
const LINGER_MS = 9000;
const REMINDER_LINGER_MS = 60_000;

export class Bubble {
  private lines: HTMLElement;
  private stopBtn: HTMLButtonElement;
  private streaming = new Map<string, HTMLElement>();
  private hideTimer = 0;
  busy = false;

  constructor(readonly el: HTMLElement, onStop: () => void) {
    this.lines = el.querySelector(".lines")!;
    this.stopBtn = el.querySelector(".stop")!;
    this.stopBtn.addEventListener("click", onStop);
  }

  get visible(): boolean {
    return !this.el.classList.contains("hidden");
  }

  private show(linger = LINGER_MS): void {
    this.el.classList.remove("hidden");
    window.clearTimeout(this.hideTimer);
    if (!this.busy) this.hideTimer = window.setTimeout(() => this.hide(), linger);
  }

  hide(): void {
    if (this.busy) return;
    this.el.classList.add("hidden");
  }

  setBusy(busy: boolean): void {
    this.busy = busy;
    this.stopBtn.classList.toggle("hidden", !busy);
    if (!busy) this.show();
    else window.clearTimeout(this.hideTimer);
  }

  private add(kind: LineKind, text: string): HTMLElement {
    const div = document.createElement("div");
    div.className = `line-${kind}`;
    div.textContent = text;
    this.lines.appendChild(div);
    while (this.lines.children.length > MAX_LINES) this.lines.firstElementChild!.remove();
    this.show(kind === "reminder" ? REMINDER_LINGER_MS : LINGER_MS);
    return div;
  }

  /** Asks for a 👍/👎 on the task that just finished (only when tasks are being saved). */
  rate(onRate: (good: boolean) => void): void {
    const row = this.add("notice", "Did that work? ");
    for (const [label, good] of [["👍", true], ["👎", false]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "rate";
      b.textContent = label;
      b.addEventListener("click", () => {
        onRate(good);
        row.textContent = good ? "Thanks! I'll learn from that." : "Noted. I'll learn from that too.";
      });
      row.appendChild(b);
    }
  }

  say(kind: LineKind, text: string): void {
    if (text.trim()) this.add(kind, plain(text));
  }

  /** A button under the latest reply (e.g. "Full answer"). */
  offer(label: string, onClick: () => Promise<unknown>): void {
    const row = this.add("notice", "");
    const b = document.createElement("button");
    b.type = "button";
    b.className = "rate offer";
    b.textContent = label;
    b.addEventListener("click", () => {
      b.disabled = true;
      onClick().then(
        () => (row.textContent = "Saved and opened."),
        (e) => (row.textContent = String(e)),
      );
    });
    row.appendChild(b);
    // Long enough to read the answer and decide.
    this.show(REMINDER_LINGER_MS);
  }

  /** A nudge with buttons (Join, Snooze, Reply…). A button's answer, if any, replaces them. */
  nudge(text: string, actions: { id: string; label: string }[], onAction: (id: string) => Promise<string | null>): void {
    const row = this.add("reminder", plain(text));
    const buttons = document.createElement("div");
    buttons.className = "nudge-actions";
    for (const a of actions) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "rate offer";
      b.textContent = a.label;
      b.addEventListener("click", () => {
        buttons.querySelectorAll("button").forEach((x) => (x.disabled = true));
        onAction(a.id).then(
          (answer) => {
            buttons.remove();
            if (answer) this.say("planner", answer);
            else if (a.id === "dismiss") this.hide();
          },
          (e) => {
            buttons.remove();
            this.say("error", String(e));
          },
        );
      });
      buttons.appendChild(b);
    }
    row.appendChild(buttons);
    this.show(REMINDER_LINGER_MS);
  }

  /** Appends streamed text to the open line for this lane, starting one if needed. */
  stream(lane: "planner" | "quick", text: string): void {
    let el = this.streaming.get(lane);
    if (!el || !el.isConnected) {
      el = this.add(lane, "");
      el.classList.add("typing");
      el.dataset.raw = "";
      this.streaming.set(lane, el);
    }
    el.dataset.raw += text;
    el.textContent = plain(el.dataset.raw ?? "");
    this.show();
  }

  endStream(lane: "planner" | "quick"): void {
    const el = this.streaming.get(lane);
    if (el) {
      el.classList.remove("typing");
      if (!el.textContent?.trim()) el.remove();
    }
    this.streaming.delete(lane);
  }

  tool(summary: string): void {
    const div = this.add("tool", "");
    div.textContent = "⚙ " + summary;
  }

  output(line: string): void {
    const last = this.lines.lastElementChild as HTMLElement | null;
    const target = last?.classList.contains("line-output") ? last : this.add("output", "");
    target.textContent = line.slice(0, 160);
    this.show();
  }

  clear(): void {
    this.lines.replaceChildren();
    this.streaming.clear();
  }

  /** Positions the bubble above (or below) a rectangle, kept on screen. */
  place(anchor: { x: number; y: number; w: number; h: number }, screen: { w: number; h: number }): void {
    if (!this.visible) return;
    const w = this.el.offsetWidth;
    const h = this.el.offsetHeight;
    const cx = anchor.x + anchor.w / 2;
    let left = Math.max(6, Math.min(screen.w - w - 6, cx - w / 2));
    let top = anchor.y - h - 14;
    const below = top < 6;
    if (below) top = anchor.y + anchor.h + 14;
    this.el.classList.toggle("below", below);
    this.el.style.setProperty("--tail-x", `${Math.max(12, Math.min(w - 12, cx - left))}px`);
    this.el.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
  }
}
