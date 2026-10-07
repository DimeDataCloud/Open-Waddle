// Builds the lab's click probes from the bench pages: renders each page at the test screen
// size (1440x960), saves the screenshot, and records a box for every visible piece of text
// ("Click \"Network & internet\"") plus the tasks' own goals ("Open Grace's email about the
// Q3 budget") with the boxes that count as right. Writes bench/lab/probes.json.
//   node bench/lab/probes.mjs
// Uses a global playwright install when it is not in node_modules: PLAYWRIGHT=/path/to/playwright/index.mjs
const { chromium } = await import(process.env.PLAYWRIGHT ?? "playwright");
import { readdirSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pages = join(here, "..", "pages");
const out = join(here, "screens");
mkdirSync(out, { recursive: true });
const W = 1440;
const H = 960;
// At most this many text probes a page, spread over small and large targets.
const PER_PAGE = 14;

const browser = await chromium.launch(process.env.CHROME ? { executablePath: process.env.CHROME } : {});
const page = await browser.newPage({ viewport: { width: W, height: H } });
const probes = [];
const tagged = {};
for (const f of readdirSync(pages).filter((f) => f.endsWith(".html")).sort()) {
  const name = f.replace(/\.html$/, "");
  await page.goto("file://" + join(pages, f));
  await page.waitForTimeout(200);
  await page.screenshot({ path: join(out, name + ".png") });
  const found = await page.evaluate(([W, H]) => {
    const box = (e) => {
      const r = e.getBoundingClientRect();
      return [r.left, r.top, r.right, r.bottom].map(Math.round);
    };
    const visible = (b) => b[2] - b[0] >= 6 && b[3] - b[1] >= 6 && b[0] >= 0 && b[1] >= 0 && b[2] <= W && b[3] <= H;
    const tags = Object.fromEntries([...document.querySelectorAll("[data-t]")].map((e) => [e.dataset.t, box(e)]));
    // Elements with text of their own (not just their children's), as a person would name them.
    const texts = [];
    for (const e of document.querySelectorAll("body *")) {
      const own = [...e.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join(" ").replace(/\s+/g, " ").trim();
      if (own.length < 2 || own.length > 40 || !/[\p{L}\p{N}]/u.test(own)) continue;
      const b = box(e);
      if (visible(b)) texts.push({ text: own, box: b });
    }
    return { tags, texts };
  }, [W, H]);
  tagged[name] = found.tags;
  // A name that appears twice on the page is ambiguous: leave it out.
  const counts = {};
  for (const t of found.texts) counts[t.text] = (counts[t.text] ?? 0) + 1;
  const unique = found.texts.filter((t) => counts[t.text] === 1);
  unique.sort((a, b) => area(a.box) - area(b.box));
  const step = Math.max(1, unique.length / PER_PAGE);
  for (let i = 0; i < unique.length && probes.filter((p) => p.screen === name).length < PER_PAGE; i += step) {
    const t = unique[Math.floor(i)];
    probes.push({ id: `${name}:${slug(t.text)}`, screen: name, kind: "text", instruction: `Click "${t.text}".`, boxes: [t.box] });
  }
}
await browser.close();

// The tasks whose first step is a click, with every box that counts.
const tasks = JSON.parse(readFileSync(join(here, "..", "tasks.json"), "utf8")).tasks;
for (const t of tasks) {
  const first = t.expect?.[0] ?? [];
  const boxes = first.map((alt) => alt.click && tagged[t.screen]?.[alt.click]).filter(Boolean);
  if (boxes.length && boxes.length === first.length) {
    probes.push({ id: `task:${t.id}`, screen: t.screen, kind: "task", instruction: t.goal, boxes });
  }
}

writeFileSync(join(here, "probes.json"), JSON.stringify({ screen: { w: W, h: H }, probes }, null, 1) + "\n");
const by = (k) => probes.filter((p) => p.kind === k).length;
console.log(`${probes.length} probes (${by("text")} named targets, ${by("task")} task goals) on ${Object.keys(tagged).length} screens`);

function area(b) {
  return (b[2] - b[0]) * (b[3] - b[1]);
}
function slug(s) {
  return s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "-").replace(/^-|-$/g, "").slice(0, 32);
}
