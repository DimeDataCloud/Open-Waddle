// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { matches, Tabs } from "./tabs";

describe("matches", () => {
  it("needs every word, in any order and case", () => {
    expect(matches("Spoken replies: read Waddle's answers aloud", "ANSWERS spoken")).toBe(true);
    expect(matches("Spoken replies", "spoken voice")).toBe(false);
    expect(matches("anything", "   ")).toBe(true);
  });
});

describe("Tabs", () => {
  function page() {
    document.body.innerHTML = `
      <input id="search" /><nav id="tabs"></nav><p id="none" class="hidden"></p>
      <form>
        <fieldset data-tab="brain"><legend>Brain</legend><input placeholder="paste your key" /></fieldset>
        <fieldset data-tab="voice"><legend>Voice</legend><select><option>Whisper-compatible API</option></select></fieldset>
        <div id="save"></div>
      </form>
      <section data-tab="diagnostics"><h2>Activity log</h2></section>`;
    const $ = (id: string) => document.getElementById(id)!;
    const tabs = new Tabs($("tabs"), $("search") as HTMLInputElement, $("none"), document, $("save"));
    const visible = () => [...document.querySelectorAll("[data-tab]:not(nav *)")].filter((e) => !e.classList.contains("hidden")).map((e) => e.querySelector("legend, h2")!.textContent);
    return { tabs, $, visible };
  }

  it("shows one tab at a time and hides Save where there's nothing to save", () => {
    const { tabs, $, visible } = page();
    expect(visible()).toEqual(["Brain"]);
    tabs.show("diagnostics");
    expect(visible()).toEqual(["Activity log"]);
    expect($("save").classList.contains("hidden")).toBe(true);
    ($("tabs").querySelector('[data-tab="voice"]') as HTMLButtonElement).click();
    expect(visible()).toEqual(["Voice"]);
    expect($("save").classList.contains("hidden")).toBe(false);
  });

  it("searches across tabs, placeholders and options included", () => {
    const { $, visible } = page();
    const search = $("search") as HTMLInputElement;
    search.value = "whisper";
    search.dispatchEvent(new Event("input"));
    expect(visible()).toEqual(["Voice"]);
    search.value = "key";
    search.dispatchEvent(new Event("input"));
    expect(visible()).toEqual(["Brain"]);
    search.value = "activity log";
    search.dispatchEvent(new Event("input"));
    expect(visible()).toEqual(["Activity log"]);
    expect($("save").classList.contains("hidden")).toBe(true);
    search.value = "zebra";
    search.dispatchEvent(new Event("input"));
    expect(visible()).toEqual([]);
    expect($("none").classList.contains("hidden")).toBe(false);
  });

  it("opens the tab of a field", () => {
    const { tabs, visible } = page();
    tabs.reveal(document.querySelector("select")!);
    expect(visible()).toEqual(["Voice"]);
  });
});
