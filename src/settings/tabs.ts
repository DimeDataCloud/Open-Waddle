// Settings tabs and search. Each fieldset or section says which tab it's on
// (data-tab); a search shows every section that matches, across all tabs.

export const TABS: { id: string; label: string }[] = [
  { id: "brain", label: "Brain" },
  { id: "assistant", label: "Assistant" },
  { id: "nudges", label: "Nudges & routines" },
  { id: "voice", label: "Voice" },
  { id: "memory", label: "Memory & privacy" },
  { id: "character", label: "Character" },
  { id: "diagnostics", label: "Diagnostics" },
];

/** Whether every word of the query is in the text (any case, any order). */
export function matches(text: string, query: string): boolean {
  const hay = text.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

/** What a section is searched by: its visible words plus its fields' placeholders. */
export function searchText(el: Element): string {
  const extra = [...el.querySelectorAll("input[placeholder], textarea[placeholder], option")].map((f) =>
    f instanceof HTMLOptionElement ? f.text : (f.getAttribute("placeholder") ?? ""),
  );
  return `${el.textContent ?? ""} ${extra.join(" ")}`;
}

export class Tabs {
  private current: string;
  private sections: HTMLElement[];

  constructor(
    private nav: HTMLElement,
    private search: HTMLInputElement,
    private noMatch: HTMLElement,
    root: ParentNode,
    /** The Save bar: hidden on tabs with nothing to save. */
    private saveBar: HTMLElement | null,
    start = "brain",
  ) {
    this.sections = [...root.querySelectorAll<HTMLElement>("[data-tab]")];
    this.current = TABS.some((t) => t.id === start) ? start : "brain";
    nav.replaceChildren(
      ...TABS.map((t) => {
        const b = document.createElement("button");
        b.type = "button";
        b.role = "tab";
        b.dataset.tab = t.id;
        b.textContent = t.label;
        b.addEventListener("click", () => this.show(t.id));
        return b;
      }),
    );
    nav.addEventListener("keydown", (e) => {
      // ← and → move between tabs.
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      const i = TABS.findIndex((t) => t.id === this.current);
      const next = TABS[(i + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length];
      this.show(next.id);
      nav.querySelector<HTMLElement>(`[data-tab="${next.id}"]`)?.focus();
    });
    search.addEventListener("input", () => this.apply());
    this.apply();
  }

  get tab(): string {
    return this.current;
  }

  show(id: string): void {
    this.current = id;
    this.search.value = "";
    this.apply();
    try {
      localStorage.setItem("settings-tab", id);
    } catch {
      // Remembering the tab is a nicety.
    }
  }

  /** Opens the tab holding `el` (e.g. a field with an error). */
  reveal(el: Element): void {
    const section = el.closest<HTMLElement>("[data-tab]");
    if (section?.dataset.tab) this.show(section.dataset.tab);
  }

  private apply(): void {
    const q = this.search.value.trim();
    let shown = 0;
    for (const s of this.sections) {
      const on = q ? matches(searchText(s), q) : s.dataset.tab === this.current;
      s.classList.toggle("hidden", !on);
      if (on) shown++;
    }
    for (const b of this.nav.querySelectorAll<HTMLButtonElement>("button")) {
      const selected = !q && b.dataset.tab === this.current;
      b.setAttribute("aria-selected", String(selected));
      b.tabIndex = selected || (q && b.dataset.tab === TABS[0].id) ? 0 : -1;
    }
    this.noMatch.classList.toggle("hidden", !q || shown > 0);
    // Save only where something saveable shows: the sections in Save's own form.
    const form = this.saveBar?.closest("form");
    const saveable = form ? this.sections.some((s) => !s.classList.contains("hidden") && form.contains(s)) : this.current !== "diagnostics";
    this.saveBar?.classList.toggle("hidden", !saveable);
  }
}
