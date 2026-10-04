// Renders each bench page in Google Chrome at the 1440x960 test screen size and
// saves a screenshot plus the boxes of every element tagged data-t="name".
//   node bench/capture.mjs
// Uses a global playwright install when it is not in node_modules: PLAYWRIGHT=/path/to/playwright/index.mjs
const { chromium } = await import(process.env.PLAYWRIGHT ?? "playwright");
import { readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "screens");
mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ executablePath: process.env.CHROME ?? "/usr/bin/google-chrome" });
const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
const boxes = {};
for (const f of readdirSync(join(here, "pages")).filter((f) => f.endsWith(".html")).sort()) {
  const name = f.replace(/\.html$/, "");
  await page.goto("file://" + join(here, "pages", f));
  await page.waitForTimeout(200);
  await page.screenshot({ path: join(out, name + ".png") });
  boxes[name] = await page.$$eval("[data-t]", (els) =>
    Object.fromEntries(els.map((e) => { const r = e.getBoundingClientRect(); return [e.dataset.t, [r.left, r.top, r.right, r.bottom].map(Math.round)]; })),
  );
}
writeFileSync(join(out, "boxes.json"), JSON.stringify(boxes, null, 1) + "\n");
await browser.close();
console.log(Object.keys(boxes).join(", "));
