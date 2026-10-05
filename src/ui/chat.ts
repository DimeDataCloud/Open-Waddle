// The chat box. Typed or spoken messages go straight to the session, even
// while a task is running (they steer it). Voice is push-to-talk:
// - "system": Windows voice typing writes live into this box; we send once
//   it goes quiet.
// - "recording": Waddle records; the second press transcribes and sends.

import type { SelectionPreview } from "../ipc";
import { Recall } from "./recall";

export interface ChatHandlers {
  /**
   * `selection`: the text grabbed with Ctrl+Alt+A goes along with this message.
   * `voice`: it was said aloud (talk mode answers those).
   */
  send(text: string, selection: boolean, voice: boolean): void;
  dropSelection(): void;
  /** The box opened: the user is about to say something. */
  opened(): void;
  voiceStart(): Promise<"system" | "recording">;
  voiceStop(): Promise<string | null>;
  error(message: string): void;
  /** The 🕘 button: open or close the conversation history. */
  history(): void;
}

const DICTATION_QUIET_MS = 1600;
const AUTO_CLOSE_MS = 30000;
/** Talk mode: how long the mic stays open for an answer before giving up. */
const LISTEN_MS = 6000;
/** With recorded voice (no live text), how long the answer may be. */
const LISTEN_RECORD_MS = 8000;
/** The box grows with what's typed, up to about six lines. */
const MAX_INPUT_PX = 120;

export class Chat {
  private input: HTMLTextAreaElement;
  /** ↑/↓ through what was said before. */
  readonly recall = new Recall();
  private mic: HTMLButtonElement;
  private voice: "off" | "system" | "recording" = "off";
  private quietTimer = 0;
  private closeTimer = 0;
  private listenTimer = 0;
  private chip: HTMLElement;
  private selection: SelectionPreview | null = null;

  constructor(readonly el: HTMLFormElement, private h: ChatHandlers) {
    this.input = el.querySelector("textarea")!;
    this.mic = el.querySelector("#mic")!;
    this.chip = el.querySelector(".chip")!;
    this.chip.querySelector("button")!.addEventListener("click", () => {
      this.setSelection(null);
      this.h.dropSelection();
      this.input.focus();
    });
    el.addEventListener("submit", (e) => {
      e.preventDefault();
      this.submit();
    });
    this.mic.addEventListener("click", () => void this.toggleVoice());
    el.querySelector("#history-btn")!.addEventListener("click", () => this.h.history());
    this.input.addEventListener("input", () => {
      this.recall.reset();
      this.fit();
      this.bumpClose();
      if (this.voice === "system" && this.input.value.trim()) {
        window.clearTimeout(this.quietTimer);
        this.quietTimer = window.setTimeout(() => this.finishDictation(), DICTATION_QUIET_MS);
      }
    });
    this.input.addEventListener("keydown", (e) => this.key(e));
  }

  get visible(): boolean {
    return !this.el.classList.contains("hidden");
  }

  open(): void {
    if (!this.visible) this.h.opened();
    this.el.classList.remove("hidden");
    this.input.focus();
    this.bumpClose();
  }

  /** Shows (or clears) the "selected text" chip above the input. */
  setSelection(sel: SelectionPreview | null): void {
    this.selection = sel;
    this.chip.classList.toggle("hidden", !sel);
    this.el.classList.toggle("with-chip", !!sel);
    if (sel) {
      const label = this.chip.querySelector("span")!;
      const from = sel.app ? ` from ${sel.app}` : "";
      label.textContent = `📎 ${sel.chars} characters${from}: “${sel.preview}${sel.chars > sel.preview.length ? "…" : ""}”`;
      this.input.placeholder = "What should I do with it?";
    } else {
      this.input.placeholder = "Ask Waddle…";
    }
  }

  close(): void {
    if (this.selection) {
      this.setSelection(null);
      this.h.dropSelection();
    }
    if (this.voice !== "off") void this.stopVoice(false);
    this.el.classList.add("hidden");
    window.clearTimeout(this.closeTimer);
  }

  toggle(): void {
    if (this.visible) this.close();
    else this.open();
  }

  private bumpClose(): void {
    window.clearTimeout(this.closeTimer);
    this.closeTimer = window.setTimeout(() => {
      if (!this.input.value.trim() && this.voice === "off") this.close();
    }, AUTO_CLOSE_MS);
  }

  private key(e: KeyboardEvent): void {
    if (e.isComposing) return;
    if (e.key === "Escape") {
      this.close();
    } else if (e.key === "Enter" && !e.shiftKey) {
      // Enter sends; Shift+Enter starts a new line.
      e.preventDefault();
      this.submit();
    } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      // Only from the first (or last) line, so moving between lines still works.
      const v = this.input.value;
      const caret = this.input.selectionStart ?? 0;
      const up = e.key === "ArrowUp";
      if (up ? v.lastIndexOf("\n", caret - 1) !== -1 : v.indexOf("\n", caret) !== -1) return;
      const text = up ? this.recall.up(v) : this.recall.down();
      if (text === null) return;
      e.preventDefault();
      this.setText(text);
    }
  }

  private setText(text: string): void {
    this.input.value = text;
    this.fit();
    const end = text.length;
    this.input.setSelectionRange(end, end);
  }

  /** Grows (or shrinks) the box to fit its text. */
  private fit(): void {
    this.input.style.height = "auto";
    this.input.style.height = `${Math.min(MAX_INPUT_PX, this.input.scrollHeight)}px`;
  }

  private submit(voice = false): void {
    const text = this.input.value.trim();
    if (!text && !this.selection) return;
    this.recall.push(text);
    this.setText("");
    this.h.send(text, !!this.selection, voice);
    this.setSelection(null);
    this.bumpClose();
  }

  private setLive(on: boolean): void {
    this.mic.classList.toggle("live", on);
    this.input.placeholder = on ? "Listening…" : "Ask Waddle…";
  }

  /** Talk mode: listens for an answer for a few seconds, then gives up quietly. */
  async listen(): Promise<void> {
    if (this.voice !== "off") return;
    await this.toggleVoice();
    if (this.voice === "off") return;
    window.clearTimeout(this.listenTimer);
    const recording = this.voice === "recording";
    this.listenTimer = window.setTimeout(
      () => {
        if (this.voice === "off") return;
        // Live dictation: nothing typed means nothing said. A recording is sent (an empty one sends nothing).
        if (recording) void this.stopVoice(true);
        else if (!this.input.value.trim()) void this.stopVoice(false);
      },
      recording ? LISTEN_RECORD_MS : LISTEN_MS,
    );
  }

  async toggleVoice(): Promise<void> {
    if (this.voice !== "off") return this.stopVoice(true);
    this.open();
    try {
      this.voice = await this.h.voiceStart();
      this.setLive(true);
    } catch (e) {
      this.voice = "off";
      this.h.error(String(e));
    }
  }

  private finishDictation(): void {
    if (this.voice !== "system") return;
    void this.stopVoice(true);
  }

  private async stopVoice(send: boolean): Promise<void> {
    const mode = this.voice;
    this.voice = "off";
    window.clearTimeout(this.quietTimer);
    window.clearTimeout(this.listenTimer);
    this.setLive(false);
    try {
      const text = await this.h.voiceStop();
      if (mode === "recording" && text) this.setText(text);
    } catch (e) {
      this.h.error(String(e));
    }
    if (send) this.submit(true);
    this.input.focus();
  }

  place(anchor: { x: number; y: number; w: number; h: number }, screen: { w: number; h: number }): void {
    if (!this.visible) return;
    const w = this.el.offsetWidth;
    const h = this.el.offsetHeight;
    const left = Math.max(6, Math.min(screen.w - w - 6, anchor.x + anchor.w / 2 - w / 2));
    let top = anchor.y + anchor.h + 8;
    if (top + h > screen.h - 6) top = anchor.y - h - 8;
    this.el.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
  }
}
