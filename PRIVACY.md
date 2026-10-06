# Privacy policy

*Last updated: 6 October 2026. Applies to the Waddle desktop app and the Waddle for Chrome extension.*

Waddle is free, open-source software that runs on your computer. There is no Waddle account and no Waddle server, and the project collects no data about you.

## Waddle for Chrome (the extension)

The extension lets the Waddle desktop app read and use web pages in Chrome when you've asked Waddle to do something.

**What it does**
- When a task you started needs it, it reads the text, links, buttons and fields of the page you point it at, and can click or type on that page.
- It passes this only to the Waddle app on **your own computer**, through Chrome's native messaging. Nothing leaves your computer through the extension.

**What it doesn't do**
- It makes no network requests of its own.
- It stores no data, and keeps no history, cookies or passwords.
- It has no analytics, tracking or advertising, and it doesn't sell or share data.
- It does nothing in the background except keep its connection to the Waddle app alive. It acts on pages only during a task you asked for.

**Permissions, and why**
- *Native messaging:* to talk to the Waddle app on this computer.
- *Tabs:* to find, open and switch tabs.
- *Scripting and access to all sites:* to read and act on whichever page you ask about. Waddle can't know in advance which site that will be.
- *Alarms:* to keep the connection to the app alive.

## The Waddle desktop app

- **Your data stays on your computer:** settings, memory, conversation history, routines, spending records and the activity log are files in your user profile. Your API keys are kept in the Windows Credential Manager (or a private file where there is no keychain).
- **AI model calls:** to do a task, Waddle sends your request, and what it needs from the screen, files, web pages or Google data for that task, to the model provider **you** chose in Settings: OpenRouter, or a model on your own computer such as Ollama. Their privacy policies then apply. With OpenRouter, Waddle asks for providers that don't keep or train on prompts. With a local model, nothing leaves your computer.
- **Google (optional):** if you connect your Google account, Waddle reads and acts on your mail, calendar, contacts and Drive on your behalf, using your own OAuth client. The sign-in token stays on your computer. Disconnect any time in Settings.
- **Updates (optional):** "Check for updates" contacts GitHub when you press it. Nothing is checked in the background.
- **No telemetry:** Waddle sends nothing to the project or anyone else except the requests above, which you set up.

## Control and deletion

- Settings → Memory & privacy shows what Waddle remembers; **Forget conversation** clears the history.
- Uninstalling Waddle and deleting its data folder removes everything it stored. Removing the Chrome extension removes the extension.

## Questions

Open an issue on the project's GitHub page. For security problems, follow [SECURITY.md](SECURITY.md).
