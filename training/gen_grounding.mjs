// Synthetic click-grounding data: opens web pages in Google Chrome, finds every
// visible button, link, field and menu item with a name, and writes one
// training example per element: a screenshot, an instruction ("Click
// Compose"), and the answer as Waddle's own click tool call, with coordinates
// on the 0-1000 grid Qwen models use. This is the recipe behind SeeClick,
// OS-Atlas and UGround: cheap, unlimited, and exactly the skill small models lack.
//
//   PLAYWRIGHT=/path/to/playwright/index.mjs node training/gen_grounding.mjs urls.txt out/
//
// urls.txt holds one URL (or local file path) per line. Don't feed it the
// bench/pages files: training on the benchmark would make its scores meaningless.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const { chromium } = await import(process.env.PLAYWRIGHT ?? "playwright");
const [listFile, outDir = "training/out/grounding"] = process.argv.slice(2);
if (!listFile) {
  console.error("usage: node training/gen_grounding.mjs urls.txt [out-dir]");
  process.exit(1);
}
const urls = readFileSync(listFile, "utf8")
  .split("\n")
  .map((l) => l.trim())
  .filter((l) => l && !l.startsWith("#"))
  .map((l) => (/^[a-z]+:\/\//.test(l) ? l : "file://" + resolve(l)));

// Screen sizes Waddle meets (logical pixels), and light/dark themes.
const SCREENS = [
  [1440, 960],
  [1280, 800],
  [1536, 864],
  [1920, 1080],
];
const PER_PAGE = Number(process.env.PER_PAGE ?? 25);

const TEMPLATES = {
  button: ["Click {n}", "Press the {n} button", "Hit {n}"],
  link: ["Open {n}", "Click the {n} link", "Go to {n}"],
  textbox: ["Click the {n} box", "Put the cursor in {n}"],
  searchbox: ["Click the search box", "Click {n}"],
  checkbox: ["Tick {n}", "Toggle {n}"],
  switch: ["Turn {n} on or off", "Toggle {n}"],
  tab: ["Switch to the {n} tab", "Open the {n} tab"],
  menuitem: ["Choose {n} from the menu", "Click {n}"],
  combobox: ["Open the {n} dropdown", "Click {n}"],
};

function rng(seed) {
  let s = seed >>> 0 || 1;
  return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
}

const imgDir = join(outDir, "images");
mkdirSync(imgDir, { recursive: true });
const browser = await chromium.launch({ executablePath: process.env.CHROME ?? "/usr/bin/google-chrome" });
const lines = [];
let shot = 0;
for (const [i, url] of urls.entries()) {
  const rand = rng(i + 1);
  const [w, h] = SCREENS[i % SCREENS.length];
  const dark = rand() < 0.3;
  const page = await browser.newPage({ viewport: { width: w, height: h }, colorScheme: dark ? "dark" : "light" });
  try {
    await page.goto(url, { waitUntil: "load", timeout: 30000 });
    await page.waitForTimeout(800);
    const targets = await page.evaluate(() => {
      const roleOf = (e) => {
        const r = e.getAttribute("role");
        if (r) return r;
        const t = e.tagName.toLowerCase();
        if (t === "a") return "link";
        if (t === "button" || (t === "input" && ["button", "submit"].includes(e.type))) return "button";
        if (t === "input" && e.type === "checkbox") return "checkbox";
        if (t === "input" && e.type === "search") return "searchbox";
        if (t === "input" || t === "textarea") return "textbox";
        if (t === "select") return "combobox";
        return null;
      };
      const nameOf = (e) =>
        (e.getAttribute("aria-label") || e.getAttribute("title") || e.innerText || e.value || e.getAttribute("placeholder") || "")
          .replace(/\s+/g, " ")
          .trim();
      const out = [];
      for (const e of document.querySelectorAll("a,button,input,textarea,select,[role],[onclick],[tabindex]")) {
        const role = roleOf(e);
        const name = nameOf(e);
        const r = e.getBoundingClientRect();
        if (!role || !name || name.length > 40 || r.width < 6 || r.height < 6) continue;
        if (r.right < 0 || r.bottom < 0 || r.left > innerWidth || r.top > innerHeight) continue;
        // Skip elements covered by something else (menus, overlays).
        const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        if (top !== e && !e.contains(top)) continue;
        out.push({ role, name, box: [r.left, r.top, r.right, r.bottom] });
      }
      return out;
    });
    // Names must be unique on the page, or the instruction is ambiguous.
    const counts = new Map();
    for (const t of targets) counts.set(t.name.toLowerCase(), (counts.get(t.name.toLowerCase()) ?? 0) + 1);
    const usable = targets.filter((t) => counts.get(t.name.toLowerCase()) === 1);
    if (!usable.length) continue;
    const file = `screen-${++shot}.png`;
    await page.screenshot({ path: join(imgDir, file) });
    for (const t of usable.sort(() => rand() - 0.5).slice(0, PER_PAGE)) {
      const tpl = (TEMPLATES[t.role] ?? ["Click {n}"])[Math.floor(rand() * (TEMPLATES[t.role]?.length ?? 1))];
      const [x0, y0, x1, y1] = t.box;
      const x = Math.round((((x0 + x1) / 2) / w) * 1000);
      const y = Math.round((((y0 + y1) / 2) / h) * 1000);
      lines.push(
        JSON.stringify({
          messages: [
            { role: "system", content: "You are Waddle, a desktop duck. Coordinates are normalised 0-1000 on both axes." },
            { role: "user", content: [{ type: "image", image: resolve(imgDir, file) }, { type: "text", text: tpl.replace("{n}", t.name) }] },
            { role: "assistant", content: "", tool_calls: [{ type: "function", function: { name: "click", arguments: JSON.stringify({ x, y }) } }] },
          ],
          // The target box on the same grid, for the GRPO click reward.
          box: [x0 / w, y0 / h, x1 / w, y1 / h].map((v) => Math.round(v * 1000)),
          source: url,
        }),
      );
    }
    console.log(`${url}: ${Math.min(usable.length, PER_PAGE)} examples`);
  } catch (e) {
    console.warn(`${url}: skipped (${e.message})`);
  } finally {
    await page.close();
  }
}
await browser.close();
writeFileSync(join(outDir, "grounding.jsonl"), lines.join("\n") + "\n");
console.log(`wrote ${lines.length} examples from ${shot} screenshots to ${join(outDir, "grounding.jsonl")}`);
