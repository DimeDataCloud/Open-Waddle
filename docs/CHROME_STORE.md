# Publishing Waddle for Chrome

What to enter in the [Chrome Web Store developer console](https://chrome.google.com/webstore/devconsole). The package is `release/waddle-chrome-<version>.zip`, made by `scripts/pack-extension.mjs`.

## Listing

- **Name:** Waddle
- **Summary (132 characters at most):** Lets Waddle, your desktop duck, read and use web pages in Chrome when you ask it to.
- **Category:** Productivity
- **Language:** English
- **Icon:** `extension/icon128.png` (128×128)
- **Screenshots:** at least one at 1280×800 or 640×400. Show the duck beside a web page with a bubble ("I filled in the form") and, ideally, the Settings → Chrome status line reading "Connected".

**Description**

> Waddle for Chrome is the browser half of Waddle, a free, open-source desktop assistant: a pixel-art duck that lives above your windows and does things for you.
>
> This extension needs the Waddle desktop app (Windows 11) to do anything. It lets Waddle read the text, links, buttons and fields of a web page, and click and type on it, so it can fill in a form, open the second search result or tell you what a page says about returns. It works from the page's real structure, so it's faster and more reliable than guessing from screenshots.
>
> - It only acts when you've asked Waddle to do something, and Waddle asks before anything risky.
> - Everything stays on your computer: the extension talks only to the Waddle app, through Chrome's native messaging. It makes no network requests, stores nothing and has no analytics.
> - Free and open source (MIT).
>
> Get the Waddle app from the project's GitHub releases page, then install this extension and press Connect in Waddle's settings.

## Privacy practices tab

- **Single purpose:** Lets the Waddle desktop app read and use web pages when you ask it to.
- **Permission justifications**
  - `nativeMessaging`: Talks to the Waddle app on this computer. This is the extension's only channel.
  - `tabs`: Finds, opens and switches tabs when a task needs another page.
  - `scripting`: Reads a page's text and fields, and clicks or types on it, for a task the user asked for.
  - `alarms`: Keeps the connection to the Waddle app alive.
  - Host permissions (all sites): Works on whichever site the user asks Waddle about. It can't be known in advance.
- **Remote code:** No. All code is in the package.
- **Data usage:** tick **Website content** (page text and links, handled only on the user's computer and passed to the Waddle app they installed). Don't tick anything else. Tick all three certifications: no sale of data, no use unrelated to the single purpose, no use for creditworthiness or lending.
- **Privacy policy URL:** the GitHub link to `PRIVACY.md` once the repository is public.

## Distribution

- Start with **Unlisted**, then move to **Public** after it works for you.
- EEA trader declaration: **non-trader** for a free hobby or open-source project published as an individual; **trader** for a company account.

## After it's approved

1. Copy the 32-letter item ID from the dashboard.
2. Add it to the extension IDs the app accepts (`EXTENSION_ID` in `src-tauri/src/browser.rs`) beside the development one, and make Settings → Chrome open the store page.
3. Each new version: bump `version` in `extension/manifest.json`, run `npm run build`, run `node scripts/pack-extension.mjs`, and upload the new zip.
