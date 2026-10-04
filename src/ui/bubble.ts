// The speech bubble: a tiny live transcript above the duck. Planner narration
// and quick replies stream into their own lines; tool activity shows as a
// status line with the latest output.

export type LineKind = "planner" | "quick" | "user" | "tool" | "output" | "notice" | "error";

const MAX_LINES = 5;
const LINGER_MS = 9000;

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

  private show(): void {
    this.el.classList.remove("hidden");
    window.clearTimeout(this.hideTimer);
    if (!this.busy) this.hideTimer = window.setTimeout(() => this.hide(), LINGER_MS);
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
    this.show();
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
    if (text.trim()) this.add(kind, text);
  }

  /** Appends streamed text to the open line for this lane, starting one if needed. */
  stream(lane: "planner" | "quick", text: string): void {
    let el = this.streaming.get(lane);
    if (!el || !el.isConnected) {
      el = this.add(lane, "");
      el.classList.add("typing");
      this.streaming.set(lane, el);
    }
    el.textContent += text;
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
