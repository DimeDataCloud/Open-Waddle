# Changelog

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
