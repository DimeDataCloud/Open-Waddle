// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { Bubble, plain } from "./bubble";

describe("plain", () => {
  it("drops the Markdown models sometimes answer in", () => {
    expect(plain("The subject is **Offsite agenda**.")).toBe("The subject is Offsite agenda.");
    expect(plain("Created `hello.txt` containing `hi`.")).toBe("Created hello.txt containing hi.");
    expect(plain("## Cost\nSee [bbc.co.uk](https://bbc.co.uk/x).")).toBe("Cost\nSee bbc.co.uk.");
    expect(plain("2 * 3 = 6, snake_case_name")).toBe("2 * 3 = 6, snake_case_name");
    expect(plain("Tips:\n*   **Short:** yes\n- Clear\n  + nested")).toBe("Tips:\n• Short: yes\n• Clear\n  • nested");
  });
});

describe("again", () => {
  it("offers one click to run a stopped request again, then goes away", () => {
    document.body.innerHTML = `<div id="bubble" class="bubble hidden"><div class="lines"></div><button class="stop hidden">Stop</button></div>`;
    const bubble = new Bubble(document.getElementById("bubble")!, () => {});
    const retry = vi.fn();
    bubble.say("notice", "Stopped (you pressed Esc). I'm not touching anything.");
    bubble.again(retry);
    const b = [...document.querySelectorAll("button")].find((x) => x.textContent === "Try again")!;
    expect(bubble.visible).toBe(true);
    b.click();
    expect(retry).toHaveBeenCalledOnce();
    expect([...document.querySelectorAll("button")].some((x) => x.textContent === "Try again")).toBe(false);
  });
});
