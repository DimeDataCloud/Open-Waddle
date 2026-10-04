# Waddle for Chrome

The extension lets Waddle read web pages and act on them by element instead of guessing from screenshots. It does nothing on its own: it only answers the Waddle desktop app, and the app only asks while it's doing a task you gave it.

- **Install:** Waddle → Settings → Chrome → **Set up Chrome extension**. Then in Chrome: `chrome://extensions` → **Developer mode** → **Load unpacked** → the folder that opened.
- **How it talks to the app:** native messaging. Chrome starts `waddle.exe` as a relay, and the relay passes messages to the running app over a named pipe (a Unix socket on Linux and macOS). No network port is opened. The app registers the relay for this extension's id only (`bjpppaeapoinfgpfgdgejapeckaflici`, fixed by the key in `manifest.json`).
- **What the app can ask:**
  - list, switch, open or close tabs
  - read a page's text, or its buttons, links and fields
  - go to an `http`/`https` address
  - find where an element is, so the duck can click it for real
  - click or type inside the page

  Page content goes to the app marked as untrusted.
- **Build:** `npm run build:extension` compiles `src/*.ts` to `build/`. The built files are committed, because the app embeds them, and CI checks they're up to date.
- **Tests:** `src/page.test.ts` runs the in-page code with jsdom (`npm test`).
