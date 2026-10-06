# Changelog

## 0.3.1: Gemini on a free key, for real

The first test with a real Google key found that tasks on Google's own API failed at their second step. These fixes were checked against Google's API with a free key.

- **Tasks on Google's API work.**
  - Gemini 3 signs each tool call it makes and refuses the next request unless the signature comes back. Waddle now keeps it and sends it back, to Google only.
  - When Gemini made two calls in one reply ("archive both emails"), Google's API sends them separately and Waddle merged them into one broken call. They now stay apart.
- **Google's limits, handled.**
  - A free key allows Gemini 3.5 Flash-Lite 15 requests a minute and gives each model a daily allowance (only 20 a day for 3.8 Flash).
  - With an OpenRouter key too, a request Google turns down goes to the same model on OpenRouter at once (`google/gemini-3.5-flash-lite`, a fraction of a cent a step). The task carries on, and Waddle goes back to the free key when Google said the limit lifts. The spending page and the monthly budget count it as usual.
  - With only a Google key, Waddle waits out a per-minute limit (Google asks for about 30 seconds) instead of failing. A used-up daily allowance is said plainly, with when it comes back, instead of "try again in a minute".
  - No AI model is involved: Google's reply says which limit was hit and for how long.
- **Token counts from Google** reach the spending page (at $0 on the free tier).
- **Checked with a free Google key:**
  - All 12 assistant jobs pass on Gemini 3.5 Flash-Lite through Google, about 2–3 seconds each.
  - The live tasks (a file, clicking from the accessibility list and from a screenshot, a web form) and the chat-lane and routing checks pass, with Flash-Lite on Google next to OpenRouter.
  - A task on 3.8 Flash after its daily allowance was used up finished on OpenRouter.
  - The benchmarks and live tests can now run on Google directly (`WADDLE_BENCH_PROVIDER=google`, `WADDLE_LIVE=google`).
- The assistant benchmark's fake Chrome now knows which page is open, so reading a page after clicking shows the page the click led to.

## 0.3.0: built from real use

Designed from three batches of the first weeks' task traces, research on desktop agents, and model tests on Waddle's own benchmark. The plan and the evidence are in [docs/BLUEPRINT-0.3.md](docs/BLUEPRINT-0.3.md).

- **Esc closes the chat box first.** While a task runs, the first Esc closes an open chat box or history drawer, as it does when Waddle is idle. The next Esc stops the task. In real use, three tasks in a row had stopped before they began, most likely from an Esc meant for the chat box.
- **Try again.** A stopped task has a **Try again** button that runs the same request again.
- **Quicker actions.**
  - Keys that only move around run straight away, without the 2-second countdown. These are snapping and switching windows, a new tab or window, the address bar, scrolling keys, back and forward, zoom and Esc.
  - Keys that could send, submit, type, paste or save still wait. In one real split-screen task, 13 key presses each waited 2 seconds.
- **Arrange windows in one step.** "Put YouTube Music on my right monitor" and "GitHub left, music right" use a new window tool, instead of key presses checked with screenshots. It snaps to either half, maximizes, minimizes, restores, brings a window forward, or moves it to another monitor.
- **Fewer steps.**
  - Opening a page or clicking on one can read the page in the same step.
  - Chrome can open a page in a new window, ready to arrange (Chrome extension 0.3.0).
  - "Search YouTube for…" goes straight to the search results: the tool knows the search addresses of YouTube, YouTube Music, GitHub, Google, Maps, Wikipedia and Amazon.
  - Waddle plans several steps in one reply when they're clear. If one fails, the rest are skipped and it looks again.
- **Misheard app names.**
  - "Open the Clawed desktop app" opens Claude: an app name with no exact match is compared with the installed apps (Start menu shortcuts and Store apps) by spelling and by sound.
  - When nothing is clearly meant, Waddle hears the closest names and asks, instead of trying a shell command that waits for approval.
  - Spoken messages lose dictation's quotation marks and the "uh"s; the history keeps what you said.
- **The duck does tricks.** "Fly around the screen", "come here", "dance", "take a nap", "hide" and "make a mess" (a pretend one, with dust; nothing on screen is touched) use a new duck tool.
- **Waddle learns what works.**
  - A task that went well leaves a short recipe of its steps, and a similar request later gets it as a hint: workflow memory, a way to improve without training a model.
  - Only what Waddle chose is kept (tool and app names, addresses without their query, keys), never typed text or anything a page or email said.
  - 👍 confirms a recipe, 👎 forgets it, and Forget conversation clears them all. The 👍/👎 question now appears after any task that taught Waddle something, not only recorded ones.
- **Gemini on a free key.**
  - Gemini 3.5 Flash-Lite passed every assistant job in Waddle's benchmark about twice as fast as the default planner (2.9 s against 5.6 s a task). It costs nothing on a free Google key.
  - **Run tasks on Gemini too** (Settings → Brain → Quick replies on Google Gemini API) runs tasks on that key, while OpenRouter keeps routing, the screen check and web searches. Without the Google key, the same model runs through OpenRouter.
  - The welcome offers **Google Gemini** as a brain, and the Google preset's planner is now Gemini 3.5 Flash-Lite.
  - Results are in [docs/MODELS.md](docs/MODELS.md).
- **Training data, version 2.** **Export training file** now writes two files:
  - `train.jsonl` for fine-tuning, without replies that claimed something no tool did, and without duplicates.
  - `kto.jsonl`: every reply of a rated task, labelled with your 👍/👎, for KTO training from thumbs alone. A new `training/finetune_kto.py` trains on it, and its `--check` validates the file without a GPU.
  - Email addresses, names in email headers, phone numbers, your user folder, saved facts and the email style note are masked in the text.
- **Checked:**
  - Unit and integration tests, frontend tests, and Windows ARM64 and x64 builds.
  - All 12 assistant jobs pass with the default model and with Gemini 3.5 Flash-Lite and 3.8 Flash.
  - The screen tasks were run with the default model and Gemini 3.5 Flash-Lite.
  - The training export was run on the real traces: nothing personal was left in the masked fields.
  - A live run of the app with the real model on a virtual screen:
    - "Left Page on the left half, Right Page on the right half" was done exactly, in 3 steps (7 s of model time, $0.0009). In the traces, a real split-screen task took 20 steps.
    - "Fly around the screen" flew the loop and landed.
    - Esc with the chat box open closed only the chat box, and the task finished. A second Esc stopped it with "Stopped (you pressed Esc)", and **Try again** ran it again.
    - A similar request got the saved recipe. One loose match ("maximize" offered a split-screen recipe) led to a stricter match.
  - Not checked here: Google's own endpoint (no key in the test environment) and Windows window placement on a real screen (it's compiled and its geometry tested). Both are in the on-device checklist.

The rest of this release:

- **No more "I've opened YouTube" when nothing happened.** In real use, the quick-reply lane and sometimes the planner said they had opened, played or searched things without doing anything, and later replies copied those lines. Now:
  - Plain requests ("Uh, can you open up YouTube?", "Go to GitHub and check the pull requests") go straight to a task instead of the chat lane. Spoken fillers and stray quotes don't get in the way.
  - A chat reply that claims an action is held back before it shows and the message becomes a task, so the false line never reaches the bubble or the conversation memory.
  - A task that claims to have done something without using a tool is told so once and asked to actually do it.
  - The planner is told that reading an email isn't showing it, and how to open an email in Chrome.
- **Waddle knows how it works.** "How do I use push-to-talk?" gets the real answer (Ctrl+Alt+Space, the microphone button), and "are you connected to my Gmail?" is answered from the actual connection. Quick replies no longer drop the date and time into answers.
- **The stop message says what stopped the task:** Esc, the Stop button, a double-click, the tray menu or saying "stop". Esc stops a running task from anywhere, so an Esc meant to close the chat box shows up plainly now.
- **Chrome actions happen in front of you.** Opening or going to a page brings that tab forward and restores a minimised Chrome window, instead of working in a tab you can't see. (Chrome extension 0.2.1.)
- **Google's Gemini API next to OpenRouter.** Settings → Brain has a **Google Gemini API** preset, and **Quick replies run on** lets chat, research summaries and the morning brief use Google (or any other OpenAI-compatible service) with its own key while the planner stays on OpenRouter. A **Test** button makes a real tool call first. Web searches still use the planner's service, and the free tier's data use is spelled out in Settings.
- **Plain error for a bad Google key** instead of raw JSON, and the self-test checks the quick-reply service too.
- **Download and code signing sections in the README,** ready for the SignPath Foundation application. They say plainly that releases aren't signed yet.
- **Chrome extension from the Web Store.** Settings → Assistant → Chrome → **Get Waddle for Chrome** opens the extension's Chrome Web Store page. The app now trusts both the store version and the folder-loaded one, so either connects. **Install from a file** still shows the Developer-mode steps for anyone who can't use the store.

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
