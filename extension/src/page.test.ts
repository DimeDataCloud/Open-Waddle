// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { clickElement, locateElement, readPage, typeInto } from "./page";

// jsdom does no layout: give elements a box from data-rect="x,y,w,h" (none = invisible).
beforeEach(() => {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    const [x, y, w, h] = (this.getAttribute("data-rect") ?? "0,0,0,0").split(",").map(Number);
    return { x, y, width: w, height: h, left: x, top: y, right: x + w, bottom: y + h, toJSON: () => ({}) } as DOMRect;
  });
  Element.prototype.scrollIntoView = () => {};
  (globalThis as { CSS?: unknown }).CSS ??= { escape: (s: string) => s };
});

function page(html: string): void {
  document.title = "Sign up";
  document.body.innerHTML = html;
}

describe("readPage", () => {
  it("lists visible controls with ids, names and values", () => {
    page(`
      <label for="email">Email address</label><input id="email" data-rect="10,10,200,30" value="me@example.com">
      <input type="password" aria-label="Password" data-rect="10,50,200,30" value="secret">
      <button data-rect="10,90,80,30">Create account</button>
      <a href="/terms" data-rect="10,1500,60,20">Terms</a>
      <button style="display:none" data-rect="0,0,10,10">Hidden</button>
      <button>No box</button>
      <input type="checkbox" aria-label="Remember me" data-rect="10,130,20,20" checked>`);
    const r = readPage("elements");
    expect(r.title).toBe("Sign up");
    expect(r.elements).toEqual([
      { id: "e1", role: "textbox", name: "Email address", value: "me@example.com" },
      { id: "e2", role: "textbox", name: "Password" },
      { id: "e3", role: "button", name: "Create account" },
      { id: "e4", role: "link", name: "Terms", offscreen: true },
      { id: "e5", role: "checkbox", name: "Remember me", value: "checked" },
    ]);
    expect(document.querySelector("button")!.getAttribute("data-waddle-id")).toBe("e3");
  });

  it("gives the main text, capped, with paragraph breaks", () => {
    page(`<nav>Menu</nav><main><h1>Ducks</h1><p>Ducks   are birds.</p><p>${"quack ".repeat(3000)}</p></main>`);
    const r = readPage("text");
    expect(r.text).toContain("Ducks are birds.");
    expect(r.text).not.toContain("Menu");
    expect(r.text!.length).toBeLessThanOrEqual(8000);
    expect(r.truncated).toBe(true);
  });
});

describe("acting on elements", () => {
  it("types the way a person would, so frameworks notice, and can submit", () => {
    page(`<form><input aria-label="Search" data-rect="0,0,100,20"></form>`);
    readPage("elements");
    const input = document.querySelector("input")!;
    const seen: string[] = [];
    input.addEventListener("input", () => seen.push(`input:${input.value}`));
    const form = document.querySelector("form")!;
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      seen.push("submit");
    });
    expect(typeInto("e1", "rubber duck", true)).toBe(true);
    expect(seen).toEqual(["input:rubber duck", "submit"]);
    expect(typeInto("e9", "x", false)).toBe(false);
  });

  it("locates an element for a real click, and clicks from inside as a fallback", () => {
    page(`<button data-rect="100,50,40,20">Go</button>`);
    readPage("elements");
    const loc = locateElement("e1")!;
    expect([loc.x, loc.y, loc.w, loc.h]).toEqual([100, 50, 40, 20]);
    expect(loc).toHaveProperty("dpr");
    let clicked = 0;
    document.querySelector("button")!.addEventListener("click", () => clicked++);
    expect(clickElement("e1")).toBe(true);
    expect(clicked).toBe(1);
    expect(locateElement("e7")).toBeNull();
  });
});
