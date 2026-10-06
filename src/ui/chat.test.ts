// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { Chat, type ChatHandlers } from "./chat";

function chat(mode: "system" | "recording", transcript: string | null = null) {
  document.body.innerHTML = `<form id="chat" class="chat hidden">
    <div class="chip hidden"><span></span><button type="button">✕</button></div>
    <textarea id="chat-input" rows="1"></textarea>
    <button type="button" id="history-btn"></button><button type="button" id="mic"></button><button type="submit" id="send"></button>
  </form>`;
  const h = {
    send: vi.fn(),
    dropSelection: vi.fn(),
    opened: vi.fn(),
    voiceStart: vi.fn(async () => mode),
    voiceStop: vi.fn(async () => transcript),
    error: vi.fn(),
    history: vi.fn(),
  } satisfies ChatHandlers;
  const c = new Chat(document.getElementById("chat") as HTMLFormElement, h);
  const input = document.querySelector("textarea")!;
  return { c, h, input };
}

const key = (el: HTMLElement, k: string, extra: KeyboardEventInit = {}) =>
  el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...extra }));

afterEach(() => vi.useRealTimers());

describe("Chat", () => {
  it("sends on Enter, keeps Shift+Enter for new lines, and recalls with ↑", () => {
    const { h, input } = chat("system");
    input.value = "first";
    key(input, "Enter");
    expect(h.send).toHaveBeenLastCalledWith("first", false, false);
    expect(input.value).toBe("");
    input.value = "line one";
    expect(key(input, "Enter", { shiftKey: true })).toBe(true);
    expect(h.send).toHaveBeenCalledTimes(1);
    input.value = "";
    key(input, "ArrowUp");
    expect(input.value).toBe("first");
  });

  it("talk mode gives up quietly when nothing is said", async () => {
    vi.useFakeTimers();
    const { c, h } = chat("system");
    await c.listen();
    expect(h.voiceStart).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(6100);
    expect(h.voiceStop).toHaveBeenCalled();
    expect(h.send).not.toHaveBeenCalled();
  });

  it("talk mode keeps listening while words arrive, and marks the message as spoken", async () => {
    vi.useFakeTimers();
    const { c, h, input } = chat("system");
    await c.listen();
    input.value = "what's next";
    input.dispatchEvent(new Event("input"));
    await vi.advanceTimersByTimeAsync(6100);
    // Dictation went quiet: it's sent as a spoken message.
    expect(h.send).toHaveBeenCalledWith("what's next", false, true);
  });

  it("a recording is sent when the listening time is up", async () => {
    vi.useFakeTimers();
    const { c, h } = chat("recording", "yes please");
    await c.listen();
    await vi.advanceTimersByTimeAsync(8100);
    expect(h.send).toHaveBeenCalledWith("yes please", false, true);
  });
});
