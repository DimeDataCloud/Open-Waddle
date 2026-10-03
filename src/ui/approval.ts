// Approval UI. Tier 3: a card that waits for an explicit click. Tier 2: a
// compact notice with a countdown and a Cancel button; the backend proceeds
// when the countdown ends (the timer authority is in Rust, not here).

import type { ApprovalRequest } from "../ipc";

export class ApprovalCard {
  current: ApprovalRequest | null = null;
  private timer = 0;

  constructor(readonly el: HTMLElement, private answer: (id: string, approved: boolean) => void) {}

  get visible(): boolean {
    return this.current !== null;
  }

  show(req: ApprovalRequest): void {
    this.current = req;
    const countdown = req.countdown_ms;
    const tier3 = countdown === null;
    this.el.className = `card ${tier3 ? "tier3" : "tier2"}`;
    this.el.replaceChildren();

    const h = document.createElement("h3");
    h.textContent = tier3 ? `Tier ${req.tier} · needs your OK` : `Tier ${req.tier} · going ahead`;
    const summary = document.createElement("div");
    summary.className = "summary";
    summary.textContent = req.summary;
    const reason = document.createElement("div");
    reason.className = "reason";
    reason.textContent = req.reason;
    this.el.append(h, summary, reason);

    if (tier3) {
      const details = document.createElement("details");
      const s = document.createElement("summary");
      s.textContent = "Details";
      const pre = document.createElement("pre");
      pre.textContent = req.detail;
      details.append(s, pre);
      this.el.append(details);
    } else {
      const bar = document.createElement("div");
      bar.className = "bar";
      const fill = document.createElement("div");
      fill.style.transition = `transform ${countdown}ms linear`;
      bar.append(fill);
      this.el.append(bar);
      requestAnimationFrame(() => requestAnimationFrame(() => (fill.style.transform = "scaleX(0)")));
    }

    const actions = document.createElement("div");
    actions.className = "actions";
    const deny = document.createElement("button");
    deny.textContent = tier3 ? "Deny" : "Cancel";
    deny.onclick = () => this.respond(false);
    const approve = document.createElement("button");
    approve.className = "approve";
    approve.textContent = tier3 ? "Approve" : "Go now";
    approve.onclick = () => this.respond(true);
    actions.append(deny, approve);
    this.el.append(actions);
    this.el.classList.remove("hidden");
    window.clearTimeout(this.timer);
    // Hide a stale countdown if the backend's resolution event never arrives.
    if (!tier3) this.timer = window.setTimeout(() => this.resolve(req.id), (countdown ?? 0) + 3000);
  }

  private respond(approved: boolean): void {
    if (!this.current) return;
    this.answer(this.current.id, approved);
    this.resolve(this.current.id);
  }

  resolve(id: string): void {
    if (this.current?.id !== id) return;
    this.current = null;
    window.clearTimeout(this.timer);
    this.el.classList.add("hidden");
  }

  place(anchor: { x: number; y: number; w: number; h: number }, screen: { w: number; h: number }, avoid: DOMRect[] = []): void {
    if (!this.visible) return;
    const w = this.el.offsetWidth;
    const h = this.el.offsetHeight;
    const clampY = (y: number) => Math.max(6, Math.min(screen.h - h - 6, y));
    const clampX = (x: number) => Math.max(6, Math.min(screen.w - w - 6, x));
    const right = anchor.x + anchor.w + 12;
    const left = anchor.x - w - 12;
    const candidates: [number, number][] = [
      [right, clampY(anchor.y + anchor.h - h)],
      [left, clampY(anchor.y + anchor.h - h)],
      [right, clampY(anchor.y)],
      [left, clampY(anchor.y)],
      [clampX(anchor.x + anchor.w / 2 - w / 2), clampY(anchor.y - h - 12)],
      [clampX(anchor.x + anchor.w / 2 - w / 2), clampY(anchor.y + anchor.h + 12)],
    ];
    // Beside the whole group (duck, bubble, chat) as a last resort.
    const groupRight = Math.max(anchor.x + anchor.w, ...avoid.map((b) => b.right)) + 12;
    const groupLeft = Math.min(anchor.x, ...avoid.map((b) => b.left)) - w - 12;
    candidates.push([groupRight, clampY(anchor.y + anchor.h - h)], [groupLeft, clampY(anchor.y + anchor.h - h)]);
    const fits = ([x, y]: [number, number]) =>
      x >= 6 && x + w <= screen.w - 6 && !avoid.some((b) => x < b.right && x + w > b.left && y < b.bottom && y + h > b.top);
    const [x, y] = candidates.find(fits) ?? [clampX(right), candidates[0][1]];
    this.el.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }
}
