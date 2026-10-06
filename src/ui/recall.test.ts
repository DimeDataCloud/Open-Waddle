import { describe, expect, it } from "vitest";
import { Recall } from "./recall";

describe("Recall", () => {
  it("steps back and forth and returns the draft", () => {
    const r = new Recall(["first", "second"]);
    expect(r.down()).toBeNull();
    expect(r.up("half typed")).toBe("second");
    expect(r.up("second")).toBe("first");
    expect(r.up("first")).toBeNull();
    expect(r.down()).toBe("second");
    expect(r.down()).toBe("half typed");
    expect(r.recalling).toBe(false);
    expect(r.down()).toBeNull();
  });

  it("adds what's sent, skipping blanks and repeats", () => {
    const r = new Recall();
    expect(r.up("")).toBeNull();
    r.push("hello");
    r.push("hello");
    r.push("   ");
    r.push("again");
    expect(r.up("")).toBe("again");
    expect(r.up("")).toBe("hello");
    expect(r.up("")).toBeNull();
    // Sending starts over from the newest.
    r.push("third");
    expect(r.up("")).toBe("third");
  });

  it("keeps the last fifty", () => {
    const r = new Recall(Array.from({ length: 60 }, (_, i) => `m${i}`));
    let oldest = "";
    for (let t = r.up(""); t !== null; t = r.up("")) oldest = t;
    expect(oldest).toBe("m10");
  });
});
