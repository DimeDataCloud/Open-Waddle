// ↑ and ↓ in the chat box step through what the user said before, like a
// shell. What they'd typed before pressing ↑ comes back after the newest one.

const KEEP = 50;

export class Recall {
  private items: string[] = [];
  /** Where ↑/↓ are; items.length means "not recalling". */
  private at = 0;
  private draft = "";

  constructor(earlier: string[] = []) {
    this.load(earlier);
  }

  /** Replaces the list (oldest first), e.g. from the saved history. */
  load(earlier: string[]): void {
    this.items = [];
    for (const t of earlier) this.push(t);
  }

  /** Something was sent: it becomes the newest item and recall starts over. */
  push(text: string): void {
    const t = text.trim();
    if (t && this.items[this.items.length - 1] !== t) this.items.push(t);
    if (this.items.length > KEEP) this.items.splice(0, this.items.length - KEEP);
    this.at = this.items.length;
    this.draft = "";
  }

  get recalling(): boolean {
    return this.at < this.items.length;
  }

  /** The text for ↑, or null when there's nothing older. */
  up(current: string): string | null {
    if (this.at === 0) return null;
    if (!this.recalling) this.draft = current;
    this.at--;
    return this.items[this.at];
  }

  /** The text for ↓, or null when not recalling. */
  down(): string | null {
    if (!this.recalling) return null;
    this.at++;
    return this.recalling ? this.items[this.at] : this.draft;
  }

  /** The user edited the text: ↑ starts from the newest again. */
  reset(): void {
    this.at = this.items.length;
  }
}
