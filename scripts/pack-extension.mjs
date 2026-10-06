// Zips the Chrome extension for the Chrome Web Store: manifest.json (without the
// development "key", because the store assigns its own ID), the icon and build/.
//
//   npm run build && node scripts/pack-extension.mjs
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const root = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const ext = join(root, "extension");
const manifest = JSON.parse(readFileSync(join(ext, "manifest.json"), "utf8"));
delete manifest.key;

const stage = join(tmpdir(), `waddle-chrome-${process.pid}`);
rmSync(stage, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
writeFileSync(join(stage, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
cpSync(join(ext, manifest.icons["128"]), join(stage, manifest.icons["128"]));
cpSync(join(ext, "build"), join(stage, "build"), { recursive: true });

mkdirSync(join(root, "release"), { recursive: true });
const out = join(root, "release", `waddle-chrome-${manifest.version}.zip`);
rmSync(out, { force: true });
// `zip` on Linux and macOS; `tar -a` (bsdtar) on Windows.
try {
  execFileSync("zip", ["-r", "-X", out, "."], { cwd: stage, stdio: "ignore" });
} catch {
  execFileSync("tar", ["-a", "-c", "-f", out, "."], { cwd: stage, stdio: "ignore" });
}
rmSync(stage, { recursive: true, force: true });
console.log(out);
