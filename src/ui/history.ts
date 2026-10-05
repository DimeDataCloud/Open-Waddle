// The history drawer: the conversation so far, beside the duck. Replies are
// rendered as Markdown (safely, see markdown.ts) with a copy button each.
// Opened from the chat box's 🕘 button or Ctrl+Alt+H.

import type { HistoryEntry } from "../ipc";
import { renderMarkdown } from "./markdown";

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface HistoryHandlers {
  list(): Promise<HistoryEntry[]>;
  openLink(url: string): Promise<void>;
  openAnswer(id: string): Promise<string>;
  /** Forgets the conversation (Waddle's memory of it too). */
  clear(): Promise<void>;
  error(message: string): void;
}

const WHO: Record<HistoryEntry["who"], string> = {
  you: "You",
  waddle: "Waddle",
  nudge: "Nudge",
  reminder: "Reminder",
  problem: "Waddle",
};

/** "14:05" today, "Mon 14:05" this week, "3 Oct 14:05" before that. */
export function when(atMs: number, now: number = Date.now()): string {
  const d = new Date(atMs);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const today = new Date(now);
  if (d.toDateString() === today.toDateString()) return time;
  if (now - atMs < 6 * 86_400_000) return `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
  return `${d.toLocaleDateString([], { day: "numeric", month: "short" })} ${time}`;
}

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Older webviews: copy through a hidden text box.
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

export class HistoryDrawer {
  private entries: HTMLElement;
  private clearBtn: HTMLButtonElement;
  private clearArmed = 0;
  private shown = "";
  private refreshing: Promise<void> | null = null;
  private again = false;

  constructor(
    readonly el: HTMLElement,
    private h: HistoryHandlers,
  ) {
    this.entries = el.querySelector(".entries")!;
    this.clearBtn = el.querySelector(".clear")!;
    el.querySelector(".close")!.addEventListener("click", () => this.close());
    this.clearBtn.addEventListener("click", () => void this.clear());
    el.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        this.close();
      }
    });
    // Links open in the browser through the backend (web links only); the drawer never navigates.
    const follow = (e: Event) => {
      const a = (e.target as Element).closest<HTMLElement>("a[data-href]");
      if (!a) return;
      e.preventDefault();
      this.h.openLink(a.dataset.href!).catch((err) => this.h.error(String(err)));
    };
    this.entries.addEventListener("click", follow);
    this.entries.addEventListener("keydown", (e) => {
      if (e.key === "Enter") follow(e);
    });
  }

  get visible(): boolean {
    return !this.el.classList.contains("hidden");
  }

  /** Opens at the newest entry. */
  async open(): Promise<void> {
    this.el.classList.remove("hidden");
    await this.refresh(true);
    // Focus for Esc and the wheel, without a ring around the whole drawer.
    this.el.focus({ preventScroll: true });
  }

  close(): void {
    this.el.classList.add("hidden");
    this.disarm();
  }

  toggle(): void {
    if (this.visible) this.close();
    else void this.open();
  }

  /** Reloads the list if the drawer is open. Calls that overlap are folded into one more load. */
  async refresh(toBottom = false): Promise<void> {
    if (!this.visible) return;
    if (this.refreshing) {
      this.again = true;
      return this.refreshing;
    }
    this.refreshing = (async () => {
      do {
        this.again = false;
        try {
          this.render(await this.h.list(), toBottom);
        } catch (e) {
          this.h.error(String(e));
        }
      } while (this.again);
    })();
    try {
      await this.refreshing;
    } finally {
      this.refreshing = null;
    }
  }

  private render(list: HistoryEntry[], toBottom: boolean): void {
    const key = list.map((e) => `${e.n}:${e.answer ?? ""}`).join(",");
    if (key === this.shown && !toBottom) return;
    this.shown = key;
    const box = this.entries;
    const atBottom = toBottom || box.scrollHeight - box.scrollTop - box.clientHeight < 24;
    box.replaceChildren(...(list.length ? list.map((e) => this.entry(e)) : [this.empty()]));
    if (atBottom) box.scrollTop = box.scrollHeight;
  }

  private empty(): HTMLElement {
    const p = document.createElement("p");
    p.className = "empty";
    p.textContent = "Nothing here yet. What you and Waddle say will show up here.";
    return p;
  }

  private entry(e: HistoryEntry): HTMLElement {
    const item = document.createElement("article");
    item.className = `entry who-${e.who}`;
    const meta = document.createElement("div");
    meta.className = "meta";
    const who = document.createElement("span");
    who.className = "who";
    who.textContent = WHO[e.who];
    const time = document.createElement("time");
    time.dateTime = new Date(e.at_ms).toISOString();
    time.textContent = when(e.at_ms);
    const copy = document.createElement("button");
    copy.type = "button";
    copy.className = "copy";
    copy.textContent = "Copy";
    copy.setAttribute("aria-label", `Copy ${WHO[e.who] === "You" ? "your message" : "this reply"}`);
    copy.addEventListener("click", () => {
      void copyText(e.text).then(() => {
        copy.textContent = "Copied";
        window.setTimeout(() => (copy.textContent = "Copy"), 1200);
      });
    });
    meta.append(who, time, copy);
    const body = document.createElement("div");
    body.className = "body";
    // What the user typed is shown as typed; replies may use Markdown.
    if (e.who === "you") body.textContent = e.text;
    else body.appendChild(renderMarkdown(e.text));
    item.append(meta, body);
    if (e.answer) {
      const id = e.answer;
      const full = document.createElement("button");
      full.type = "button";
      full.className = "offer";
      full.textContent = "Full answer";
      full.addEventListener("click", () => {
        full.disabled = true;
        this.h.openAnswer(id).then(
          () => (full.textContent = "Saved and opened"),
          (err) => {
            full.disabled = false;
            this.h.error(String(err));
          },
        );
      });
      item.appendChild(full);
    }
    return item;
  }

  private disarm(): void {
    window.clearTimeout(this.clearArmed);
    this.clearArmed = 0;
    this.clearBtn.textContent = "Forget";
    this.clearBtn.classList.remove("armed");
  }

  /** Two clicks: the first asks "Sure?". */
  private async clear(): Promise<void> {
    if (!this.clearArmed) {
      this.clearBtn.textContent = "Sure?";
      this.clearBtn.classList.add("armed");
      this.clearArmed = window.setTimeout(() => this.disarm(), 3000);
      return;
    }
    this.disarm();
    try {
      await this.h.clear();
      await this.refresh(true);
    } catch (e) {
      this.h.error(String(e));
    }
  }

  /** Where the drawer sits while open (its bottom edge, so it grows upward). It stays put unless it would cover something or leave the screen. */
  private pos: { x: number; bottom: number } | null = null;

  /**
   * Beside the duck and its bubble and chat box (`group`), wherever there's room.
   * `avoid` are the boxes it mustn't cover (the duck, bubble, chat box).
   */
  place(group: Box, screen: { w: number; h: number }, avoid: Box[] = []): void {
    if (!this.visible) {
      this.pos = null;
      return;
    }
    const w = this.el.offsetWidth;
    const h = this.el.offsetHeight;
    const clampY = (y: number) => Math.max(6, Math.min(screen.h - h - 6, y));
    const covers = (x: number, y: number) => avoid.some((b) => x < b.x + b.w && x + w > b.x && y < b.y + b.h && y + h > b.y);
    let x = this.pos?.x ?? -1;
    let y = this.pos ? clampY(this.pos.bottom - h) : -1;
    if (!this.pos || x < 6 || x + w > screen.w - 6 || covers(x, y)) {
      y = clampY(group.y + group.h - h);
      if (group.x + group.w + 12 + w <= screen.w - 6) x = group.x + group.w + 12;
      else if (group.x - 12 - w >= 6) x = group.x - 12 - w;
      else x = Math.max(6, Math.min(screen.w - w - 6, group.x + group.w / 2 - w / 2));
    }
    this.pos = { x, bottom: y + h };
    this.el.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }
}
