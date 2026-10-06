// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import type { ApprovalRequest } from "../ipc";
import { ApprovalCard } from "./approval";

const req = (countdown: number | null): ApprovalRequest => ({
  id: "a1",
  task_id: "t1",
  tier: countdown === null ? 3 : 2,
  tool: "run_command",
  summary: "Run `rm notes.txt`",
  reason: "deletes a file",
  detail: "{}",
  countdown_ms: countdown,
});

function card() {
  document.body.innerHTML = `<div id="approval" class="card hidden"></div>`;
  const answer = vi.fn();
  const c = new ApprovalCard(document.getElementById("approval")!, answer);
  return { c, answer, el: document.getElementById("approval")! };
}

const press = (el: HTMLElement, key: string) => el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));

describe("ApprovalCard", () => {
  it("is announced, and Esc denies", () => {
    const { c, answer, el } = card();
    c.show(req(null));
    expect(el.getAttribute("role")).toBe("alertdialog");
    expect(el.getAttribute("aria-label")).toContain("Needs your OK: Run `rm notes.txt`");
    press(el, "Escape");
    expect(answer).toHaveBeenCalledWith("a1", false, undefined);
  });

  it("has no Enter shortcut to approve, and doesn't take focus", () => {
    const { c, answer, el } = card();
    c.show(req(null));
    press(el, "Enter");
    expect(answer).not.toHaveBeenCalled();
    expect(el.contains(document.activeElement)).toBe(false);
  });

  it("a countdown is a polite status", () => {
    const { c, el } = card();
    c.show(req(2000));
    expect(el.getAttribute("role")).toBe("status");
    expect(el.getAttribute("aria-live")).toBe("polite");
  });
});
