# Changelog

## 0.2.9: release candidate

- **Ready to open source.** MIT license file, a contributing guide (what CI checks, ground rules for safety tiers and untrusted text) and a security policy with private reporting.
- **Updates from inside Waddle.** Settings → Diagnostics → **Check for updates** looks on the GitHub releases page, and **Install and restart** downloads the new version and checks its signature against a key built into the app before installing. It only happens when you press the buttons: nothing is checked in the background, and Waddle can't start it by itself. Release builds sign the update files and publish `latest.json` beside the installers.
- **Ready for the Chrome Web Store.** A privacy policy (`PRIVACY.md`), the listing text and permission justifications (`docs/CHROME_STORE.md`), and `npm run pack:extension`, which zips the extension without its development key.
- **Installers for x64 too.** Pushing a version tag builds Windows installers for ARM64 (Snapdragon) and x64 (Intel and AMD) and attaches them to a draft GitHub release.
- **The welcome offers Google and Chrome.** Optional "Set up…" links, ticked when already done, finish the welcome and open that section of Settings.
- **High contrast.** With a Windows contrast theme on, the bubble, chat box, cards, history and Settings use the theme's colours, with borders where colour alone told things apart.
- **Settings search** hides Save when only Diagnostics matches.
- **Self-test covers more.** It now reports how many monitors there are and spending against the monthly budget, alongside MCP servers, routines and voices. A microphone that can't be opened says so in words.
- **Finds "the newsletters".** Gmail search matches words literally, so a search for "newsletter" often finds nothing. Waddle is now told to look through the inbox and judge by sender and subject instead of giving up.
- **README rewritten around what Waddle does,** with an on-device checklist of 43 steps grouped by area (first run, the duck, tasks and safety, conversation and voice, spending, Google, Chrome, files, routines and MCP).
- **Checked:** all unit and integration tests; all 12 assistant benchmark tasks (mail, calendar, files, documents, Chrome, memory, delete guard) with the default model, at about $0.0005 and 5 s per task (tasks that hit OpenRouter's rate limit were re-run); routing, the screen check and mail importance (29 of 30) with the decision model; Windows ARM64 and x64 builds; no known vulnerabilities in npm or Rust dependencies.

## 0.2.8: polish

- **A welcome on first run.** With no brain set up yet, Waddle opens a short welcome.
  - Paste an OpenRouter key and press **Test** (one tiny call), let Waddle find Ollama on this computer, or just look around in demo mode.
  - Pick a colour, and you're done. It never appears again once finished or skipped.
- **Settings in tabs, with search.** Brain, Assistant, Nudges & routines, Voice, Memory & privacy, Character and Diagnostics. Type in the search box to find any setting across tabs (field hints and options count). Save stays in view at the bottom, and a required field on another tab opens that tab instead of silently blocking Save. The last tab is remembered.
- **Little effects.** A puff of dust when the duck lands, a sparkle when a task is done, a sweat drop when one fails, a "?" when Waddle asks you something, and a heart when you say thanks. With Windows' reduce-motion setting on, only still symbols show.
- **Keyboard and screen readers.**
  - Esc denies an approval card. There's no Enter shortcut to approve, and the card never takes focus by itself, so Enter typed in the chat box can't approve an action.
  - Cards are announced: "needs your OK" urgently, countdowns politely.
  - Visible focus rings across the overlay and Settings; ← and → move between Settings tabs.
- **Clearer microphone errors.** "I can't find a microphone" or "I couldn't start the microphone", with where to fix it in Windows, instead of "microphone config".

## 0.2.7: routines

- **Routines: tasks Waddle does on a schedule.** "Every weekday at 8:45, summarise my unread email." "Every Friday at 4pm, list the files I changed this week." "Today at 6pm, draft my weekly report."
  - Ask in chat (it needs your click to set up), or add them in Settings → **Nudges & routines**. Pause, resume or delete them there too, or ask.
  - When one is due, the duck says so ("🔁 Routine…") and does it. The result shows in the bubble and the conversation history, and is read aloud if spoken replies are on.
- **Safe while you're busy or away.**
  - A routine never touches the screen, mouse or keyboard. It uses email, calendar, files and connected tools.
  - Every step that would change something waits for your click; there are no countdowns.
  - A routine can't set up more routines.
- **Never late.** If Waddle wasn't running at the time (the computer was off), the routine is marked missed and runs next time. Waddle says so; it doesn't do a morning's routine at midday. If a task is running when one comes due, the routine waits for it.
- **Safer tool calls everywhere.** Waddle now refuses any tool it wasn't offered for the current task (for example a made-up tool name), instead of trying to run it.

## 0.2.6: MCP tools

- **Waddle can use MCP servers.** Add the MCP servers of apps and services you use (GitHub, Notion, a folder of notes, a database…) in the new Settings → **Tools (MCP)** section, and their tools are offered to every task. See [docs/MCP.md](docs/MCP.md).
  - Type a name and command, or paste the JSON block from the server's instructions and press **Import**.
  - **Test** starts the server once and lists its tools.
  - Keys a server needs go in the keychain, not the settings file.
- **Safe by default.**
  - Every tool of a server you haven't marked trusted waits for your click; tools that say they only read get a countdown.
  - On trusted servers, read-only tools run at once and others after a countdown, or a click if they say they may delete things.
  - Tool answers are untrusted text, and Waddle can't add, change or remove servers.
- **Light on resources.** A server starts only when a task first uses one of its tools and stops after 10 idle minutes. Tool lists are remembered, so tasks don't wait for servers to start. A server that crashed is started again for the next call. Stop halts a slow tool at once.

## 0.2.5: spoken replies

- **Waddle can read its answers aloud.** Settings → Voice → **Spoken replies** (off by default).
  - It reads answers: chat replies, research summaries, quick replies during a task, and a task's last words. It doesn't read every step.
  - Optionally it reads nudges and reminders too.
  - Pick any installed Windows voice and a speed, and press **Test voice**.
  - Formatting, emoji, web addresses and citations are left out, and long answers stop after a few sentences (the rest stays on screen).
- **It knows when to be quiet.** No speaking in full screen or during a meeting on your calendar (with Google connected and meeting nudges on). Opening the chat box, talking, sending a message or Stop cuts it off.
- **Talk mode.** After answering something you said aloud, Waddle listens again for 6 seconds, so a conversation can go back and forth hands-free. If you say nothing, the mic closes quietly.
- You can also ask Waddle to "read your answers aloud"; it changes the setting with your OK.
- The self-test lists the voices it found.

## 0.2.4: conversation panel

- **Conversation history.** 🕘 in the chat box (or **Ctrl+Alt+H**) opens the last 50 messages beside the duck: what you said, Waddle's replies, research summaries (with their **Full answer** button), nudges, reminders and the morning brief. It stays put while you read and survives restarts.
  - Replies are formatted: lists, bold, code and links. Links open in your browser (web links only).
  - Each message has a **Copy** button.
  - **Forget** (two clicks) clears it along with Waddle's memory of the conversation, as does Settings → Memory & privacy → **Forget conversation**.
- **Long replies fold.** The bubble shows the newest words while a long reply streams, then its start and a **More…** button that opens the history.
- **A better chat box.**
  - ↑ and ↓ bring back what you said before, including from earlier runs.
  - It grows as you type. Enter sends; Shift+Enter starts a new line.
- Lists in the bubble show as • bullets instead of Markdown asterisks.
- Waddle never presses Ctrl+Alt+H itself (like your other Waddle shortcuts).

## 0.2.3: multiple monitors

- **Waddle follows you across monitors.** When you've been working in a window on another monitor for a couple of seconds, the duck moves there and drops in from the side you came from. During a task it stays put. Settings → Character → "Follow me across monitors" (on by default); off keeps it on the main monitor.
- **Everything follows the duck's monitor:** screenshots, window positions, clicks and scaling, including a 200% Surface screen next to a 100% external one.
- **Clicks on other monitors land correctly on Windows.** The input library only scaled pointer moves to the main monitor; points elsewhere now use the whole virtual desktop.
- **The model knows what it can't see.** Windows on another monitor are listed as "on another screen" instead of with coordinates off the screenshot.
- On a monitor other than the main one, clicks on web pages happen inside the page, because Chrome's screen coordinates are only reliable on the main monitor.
- If the duck's monitor is unplugged, it returns to the main one.

## 0.2.2: leaner, faster, cheaper

- **Spending meter and monthly budget.**
  - Every paid model call is recorded by purpose (tasks, chat, research, quick replies, briefs, decisions, self-tests), using the cost OpenRouter reports.
  - Settings → Spending shows today, this month, the last 30 days and the breakdown.
  - The monthly budget (default $5) warns once at 80%. At 100% paid calls pause until the 1st; stop, reminders and the screen keep working, and local models are never paused.
- **Chat replies stream.** The first words appear in about 0.7 s instead of after the whole reply. A reply that turns out to be a task still hands off cleanly.
- **Long tasks stay lean.** Once a task's conversation passes about 48,000 characters, earlier long tool results (file reads, command output) are cut down to their start, so later steps don't pay for them again. The latest results always stay whole.
- **The conversation survives restarts.** The last 10 exchanges are kept, and forgotten after 12 hours of quiet. Settings → Memory & privacy → **Forget conversation** clears them now.
- **Battery-aware.** Unplugged (or in battery saver), Waddle samples windows 4 times a second instead of 10 and makes ambient decisions half as often. The overlay also draws fewer idle frames.

## 0.2.1: reliability

- **One Waddle at a time.** Opening Waddle while it's already running (or autostart racing a manual start) brings up the running duck's chat box instead of a second duck.
- **Model calls survive hiccups.**
  - Rate limits, server errors and dropped connections are retried up to twice on the same model, but only before anything has been shown.
  - A stream that goes quiet for 60 s (5 minutes for local models) counts as dropped.
  - Errors say what to do next, for example "the OpenRouter account is out of credit. Add some at openrouter.ai/credits".
- **No more cut-off replies.** The planner may write up to 4,096 tokens per step (was 1,024). A reply that still hits the limit is never run as a half-written tool call; the model is asked to write shorter content or split it.
- **Follows display changes.** Rotating the Surface, docking, changing the resolution or scale, or moving the taskbar re-places the overlay within about 2 s, and the duck lands back on screen.
- **Crash-safe saves.**
  - Settings, facts, reminders, skills, the style note, nudges and saved keys are written to a temporary file and swapped in, so a crash can't leave half a file.
  - A damaged settings file is kept as `settings.json.bad` and Waddle says so, instead of silently starting over.
  - The nudge state is only written when it changes.
- **Expired Google sign-in.** When Google signs Waddle out, it stops polling, says so once, and Settings shows "Sign-in expired" with **Connect** to sign in again.
- **Shortcut safety doesn't depend on spelling.** `F4+Alt`, `Control+W`, `Win+R` and `Ctrl+Alt+Del` are recognised like their usual spellings. `Ctrl+F4` now needs a click too.
- **Research answers keep their files.** Asking the same question again saves `…-2.md` instead of replacing the earlier answer.
