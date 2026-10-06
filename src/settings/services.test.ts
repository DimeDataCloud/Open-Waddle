import { describe, expect, it } from "vitest";
import { fastDefaults, fastServiceFor, GOOGLE_QUICK, GOOGLE_URL, usesGoogle } from "./services";

describe("quick-reply service", () => {
  it("is read from the saved address", () => {
    expect(fastServiceFor("")).toBe("same");
    expect(fastServiceFor("  ")).toBe("same");
    expect(fastServiceFor(GOOGLE_URL)).toBe("google");
    expect(fastServiceFor("https://example.com/v1")).toBe("custom");
  });

  it("picks Google's address and a Gemini model, keeping a Gemini model already chosen", () => {
    expect(fastDefaults("google", "google/gemini-2.5-flash-lite", "", "google/gemini-2.5-flash-lite")).toEqual({ url: GOOGLE_URL, model: GOOGLE_QUICK });
    expect(fastDefaults("google", "", "", "gemini-3.8-flash")).toEqual({ url: GOOGLE_URL, model: "gemini-3.8-flash" });
  });

  it("goes back to the planner's service and its suggested model", () => {
    expect(fastDefaults("same", "google/gemini-2.5-flash-lite", GOOGLE_URL, GOOGLE_QUICK)).toEqual({ url: "", model: "google/gemini-2.5-flash-lite" });
  });

  it("'another service' keeps an address but never Google's", () => {
    expect(fastDefaults("custom", "x", "https://example.com/v1", "m")).toEqual({ url: "https://example.com/v1", model: "m" });
    expect(fastDefaults("custom", "x", GOOGLE_URL, "m").url).toBe("");
  });

  it("knows when Google is in use, as planner or for quick replies", () => {
    expect(usesGoogle("https://openrouter.ai/api/v1", "")).toBe(false);
    expect(usesGoogle(GOOGLE_URL, "")).toBe(true);
    expect(usesGoogle("https://openrouter.ai/api/v1", GOOGLE_URL)).toBe(true);
  });
});
