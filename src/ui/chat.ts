// The chat box. Typed or spoken messages go straight to the session, even
// while a task is running (they steer it). Voice is push-to-talk:
// - "system": Windows voice typing writes live into this box; we send once
//   it goes quiet.
// - "recording": Waddle records; the second press transcribes and sends.

export interface ChatHandlers {
  send(text: string): void;
  voiceStart(): Promise<"system" | "recording">;
  voiceStop(): Promise<string | null>;
  error(message: string): void;
}

const DICTATION_QUIET_MS = 1600;
const AUTO_CLOSE_MS = 30000;

export class Chat {
  private input: HTMLInputElement;
  private mic: HTMLButtonElement;
  private voice: "off" | "system" | "recording" = "off";
  private quietTimer = 0;
  private closeTimer = 0;

  constructor(readonly el: HTMLFormElement, private h: ChatHandlers) {
    this.input = el.querySelector("input")!;
    this.mic = el.querySelector("#mic")!;
    el.addEventListener("submit", (e) => {
      e.preventDefault();
      this.submit();
    });
    this.mic.addEventListener("click", () => void this.toggleVoice());
    this.input.addEventListener("input", () => {
      this.bumpClose();
      if (this.voice === "system" && this.input.value.trim()) {
        window.clearTimeout(this.quietTimer);
        this.quietTimer = window.setTimeout(() => this.finishDictation(), DICTATION_QUIET_MS);
      }
    });
    this.input.addEventListener("keydown", (e) => {
      if (e.key === "Escape") this.close();
    });
  }

  get visible(): boolean {
    return !this.el.classList.contains("hidden");
  }

  open(): void {
    this.el.classList.remove("hidden");
    this.input.focus();
    this.bumpClose();
  }

  close(): void {
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

  private submit(): void {
    const text = this.input.value.trim();
    if (!text) return;
    this.input.value = "";
    this.h.send(text);
    this.bumpClose();
  }

  private setLive(on: boolean): void {
    this.mic.classList.toggle("live", on);
    this.input.placeholder = on ? "Listening…" : "Ask Waddle…";
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
    this.setLive(false);
    try {
      const text = await this.h.voiceStop();
      if (mode === "recording" && text) this.input.value = text;
    } catch (e) {
      this.h.error(String(e));
    }
    if (send) this.submit();
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
