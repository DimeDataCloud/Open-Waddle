// Builds latest.json, the file installed copies of Waddle read to find an update.
//
//   node scripts/make-latest-json.mjs <dir-with-installers-and-.sig-files> <tag> <owner/repo> [notes]
//
// Each installer (Waddle_<version>_<arch>-setup.exe) must sit beside its signature
// (…-setup.exe.sig), made by `tauri build` when TAURI_SIGNING_PRIVATE_KEY is set.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [dir, tag, repo, notes = ""] = process.argv.slice(2);
if (!dir || !tag || !repo) {
  console.error("usage: make-latest-json.mjs <dir> <tag> <owner/repo> [notes]");
  process.exit(2);
}

const ARCH = { arm64: "windows-aarch64", x64: "windows-x86_64" };
const platforms = {};
for (const name of readdirSync(dir).filter((n) => n.endsWith("-setup.exe"))) {
  const arch = name.match(/_(arm64|x64)-setup\.exe$/)?.[1];
  if (!arch) continue;
  const signature = readFileSync(join(dir, `${name}.sig`), "utf8").trim();
  platforms[ARCH[arch]] = { signature, url: `https://github.com/${repo}/releases/download/${tag}/${name}` };
}
if (Object.keys(platforms).length === 0) {
  console.error(`no signed installers found in ${dir}`);
  process.exit(1);
}

const manifest = { version: tag.replace(/^v/, ""), notes, pub_date: new Date().toISOString(), platforms };
writeFileSync(join(dir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
console.log(`latest.json for ${manifest.version}: ${Object.keys(platforms).join(", ")}`);
