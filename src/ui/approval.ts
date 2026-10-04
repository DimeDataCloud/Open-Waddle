// Approval UI. Tier 3: a card that waits for an explicit click. Tier 2: a
// compact notice with a countdown and a Cancel button; the backend proceeds
// when the countdown ends (the timer authority is in Rust, not here).
// Emails get a send card (the whole message, editable) and, after Send, an
// Undo bar while the backend holds the message back.

import type { ApprovalRequest, MailDraft } from "../ipc";

const FIELDS = [
  ["to", "To"],
  ["cc", "Cc"],
  ["subject", "Subject"],
] as const;

export class ApprovalCard {
  current: ApprovalRequest | null = null;
  private undoId: string | null = null;
  private timer = 0;

  constructor(
    readonly el: HTMLElement,
    private answer: (id: string, approved: boolean, draft?: MailDraft) => void,
    private undo: (id: string) => void = () => {},
  ) {}

  get visible(): boolean {
    return this.current !== null || this.undoId !== null;
  }

  show(req: ApprovalRequest): void {
    if (req.draft) {
      this.showDraft(req, req.draft);
      return;
    }
    this.undoId = null;
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

  /** The send card: the full message, read-only until Edit; Send returns it as shown. */
  private showDraft(req: ApprovalRequest, draft: MailDraft): void {
    this.undoId = null;
    this.current = req;
    window.clearTimeout(this.timer);
    this.el.className = "card tier3 mail";
    this.el.replaceChildren();
    const h = document.createElement("h3");
    h.textContent = "Send this email?";
    const form = document.createElement("div");
    form.className = "mail-fields";
    const inputs = new Map<keyof MailDraft, HTMLInputElement | HTMLTextAreaElement>();
    for (const [key, label] of FIELDS) {
      if (key === "cc" && !draft.cc) continue;
      const row = document.createElement("label");
      row.textContent = label;
      const input = document.createElement("input");
      input.value = draft[key] ?? "";
      input.readOnly = true;
      inputs.set(key, input);
      row.append(input);
      form.append(row);
    }
    const body = document.createElement("textarea");
    body.value = draft.body;
    body.readOnly = true;
    body.rows = Math.min(12, Math.max(4, draft.body.split("\n").length + 1));
    inputs.set("body", body);
    form.append(body);

    const actions = document.createElement("div");
    actions.className = "actions";
    const cancel = document.createElement("button");
    cancel.textContent = "Cancel";
    cancel.onclick = () => this.respond(false);
    const edit = document.createElement("button");
    edit.textContent = "Edit";
    edit.onclick = () => {
      for (const input of inputs.values()) input.readOnly = false;
      this.el.classList.add("editing");
      body.focus();
      edit.remove();
    };
    const send = document.createElement("button");
    send.className = "approve";
    send.textContent = "Send";
    send.onclick = () => {
      const value = (k: keyof MailDraft) => inputs.get(k)?.value ?? "";
      this.respond(true, { ...draft, to: value("to"), cc: inputs.has("cc") ? value("cc") : draft.cc, subject: value("subject"), body: value("body") });
    };
    actions.append(cancel, edit, send);
    this.el.append(h, form, actions);
    this.el.classList.remove("hidden");
  }

  /** After Send: the email waits `secs` seconds in case the user wants it back. */
  showUndo(id: string, secs: number): void {
    this.current = null;
    this.undoId = id;
    window.clearTimeout(this.timer);
    this.el.className = "card undo";
    this.el.replaceChildren();
    const text = document.createElement("span");
    let left = secs;
    const tick = () => (text.textContent = `Sending in ${left} s…`);
    tick();
    const button = document.createElement("button");
    button.className = "approve";
    button.textContent = "Undo";
    button.onclick = () => {
      this.undo(id);
      this.hideUndo(id);
    };
    this.el.append(text, button);
    this.el.classList.remove("hidden");
    const step = () => {
      left -= 1;
      if (this.undoId !== id) return;
      if (left <= 0) {
        text.textContent = "Sending…";
        return;
      }
      tick();
      this.timer = window.setTimeout(step, 1000);
    };
    this.timer = window.setTimeout(step, 1000);
  }

  hideUndo(id: string): void {
    if (this.undoId !== id) return;
    this.undoId = null;
    window.clearTimeout(this.timer);
    this.el.classList.add("hidden");
  }

  private respond(approved: boolean, draft?: MailDraft): void {
    if (!this.current) return;
    this.answer(this.current.id, approved, draft);
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
