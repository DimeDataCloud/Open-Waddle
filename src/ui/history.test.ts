// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import type { HistoryEntry } from "../ipc";
import { HistoryDrawer, when } from "./history";

function drawer(list: HistoryEntry[]) {
  document.body.innerHTML = `<section id="history" class="drawer hidden">
    <header><strong>Conversation</strong><button class="clear">Forget</button><button class="close">✕</button></header>
    <div class="entries" tabindex="0"></div></section>`;
  const h = {
    list: vi.fn(async () => list),
    openLink: vi.fn(async () => {}),
    openAnswer: vi.fn(async () => "saved.md"),
    clear: vi.fn(async () => {
      list = [];
    }),
    error: vi.fn(),
  };
  return { d: new HistoryDrawer(document.getElementById("history")!, h), h };
}

const at = Date.now();

describe("HistoryDrawer", () => {
  it("shows what the user typed as typed and replies as Markdown", async () => {
    const { d, h } = drawer([
      { n: 1, at_ms: at, who: "you", text: "**not bold** <i>x</i>" },
      { n: 2, at_ms: at, who: "waddle", text: "See **this**: [docs](https://docs.example)", answer: "answer_1" },
      { n: 3, at_ms: at, who: "problem", text: "Stopped." },
    ]);
    await d.open();
    expect(d.visible).toBe(true);
    const entries = document.querySelectorAll(".entry");
    expect(entries).toHaveLength(3);
    expect(entries[0].querySelector(".body")!.textContent).toBe("**not bold** <i>x</i>");
    expect(entries[0].querySelector("strong, i")).toBeNull();
    expect(entries[1].querySelector("strong")!.textContent).toBe("this");
    expect(entries[2].classList.contains("who-problem")).toBe(true);

    (entries[1].querySelector("a") as HTMLElement).click();
    expect(h.openLink).toHaveBeenCalledWith("https://docs.example/");
    (entries[1].querySelector(".offer") as HTMLButtonElement).click();
    expect(h.openAnswer).toHaveBeenCalledWith("answer_1");
  });

  it("asks before forgetting, and says when there's nothing", async () => {
    const { d, h } = drawer([{ n: 1, at_ms: at, who: "you", text: "hi" }]);
    await d.open();
    const clear = document.querySelector(".clear") as HTMLButtonElement;
    clear.click();
    expect(h.clear).not.toHaveBeenCalled();
    expect(clear.textContent).toBe("Sure?");
    clear.click();
    await vi.waitFor(() => expect(document.querySelector(".empty")).not.toBeNull());
    expect(h.clear).toHaveBeenCalledTimes(1);
    expect(clear.textContent).toBe("Forget");
  });

  it("closes with Esc and doesn't load while closed", async () => {
    const { d, h } = drawer([]);
    await d.refresh();
    expect(h.list).not.toHaveBeenCalled();
    await d.open();
    document.querySelector(".entries")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(d.visible).toBe(false);
  });
});

describe("HistoryDrawer.place", () => {
  it("stays put while open unless it would cover the duck's things", async () => {
    const { d } = drawer([]);
    Object.defineProperty(d.el, "offsetWidth", { value: 340 });
    Object.defineProperty(d.el, "offsetHeight", { value: 200, configurable: true });
    const screen = { w: 1440, h: 960 };
    const duck = { x: 1000, y: 900, w: 64, h: 56 };
    const bubble = { x: 850, y: 800, w: 320, h: 90 };
    const pos = () => d.el.style.transform;
    await d.open();
    // With the bubble up, beside the whole group: the right doesn't fit, so the left.
    d.place({ x: 850, y: 800, w: 320, h: 156 }, screen, [duck, bubble]);
    expect(pos()).toBe("translate(498px, 754px)");
    // The bubble goes away: no jump.
    d.place(duck, screen, [duck]);
    expect(pos()).toBe("translate(498px, 754px)");
    // Fewer entries: it shrinks toward its bottom edge, staying by the duck.
    Object.defineProperty(d.el, "offsetHeight", { value: 80, configurable: true });
    d.place(duck, screen, [duck]);
    expect(pos()).toBe("translate(498px, 874px)");
    Object.defineProperty(d.el, "offsetHeight", { value: 200, configurable: true });
    // The duck walks under it: it moves.
    const moved = { x: 600, y: 900, w: 64, h: 56 };
    d.place(moved, screen, [{ ...moved, y: 760 }]);
    expect(pos()).toBe("translate(676px, 754px)");
    // Reopened, it starts fresh.
    d.close();
    d.place(duck, screen, [duck]);
    await d.open();
    d.place(duck, screen, [duck]);
    expect(pos()).toBe("translate(1076px, 754px)");
  });
});

describe("when", () => {
  it("shows only the time for today and adds the day before that", () => {
    const now = new Date(2026, 9, 5, 15, 0).getTime();
    const today = when(new Date(2026, 9, 5, 9, 30).getTime(), now);
    expect(today).toMatch(/9.?30/);
    expect(today.length).toBeLessThan(9);
    expect(when(new Date(2026, 9, 3, 9, 30).getTime(), now).length).toBeGreaterThan(today.length);
    expect(when(new Date(2026, 8, 1, 9, 30).getTime(), now)).toMatch(/Sep|1/);
  });
});
