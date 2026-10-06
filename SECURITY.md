# Security policy

Waddle acts on your computer for you, so security problems matter. Thank you for reporting them responsibly.

## Reporting a vulnerability

Please **don't open a public issue**. Use GitHub's private reporting instead: the repository's **Security** tab → **Report a vulnerability**. Include what an attacker needs, what they can do, and steps to reproduce. We aim to reply within a week and to fix serious problems before anything is made public.

Especially interesting:
- a way for text on screen, in a file, a web page, an email or an MCP tool result to make Waddle act without the approval its tier requires;
- anything that approves an action without a click in Waddle's own window;
- a way to read or change API keys, endpoints, allowed folders, the budget or MCP servers through the model;
- escaping the allowed folders with file tools, or running commands the tier rules should stop;
- tampering with the activity log without `Verify integrity` noticing;
- the Chrome extension or native messaging host being usable by other extensions or pages.

## What Waddle doesn't claim

- Commands run in the workspace folder, but this is **not an OS sandbox**. The Tier 3 click is the real gate.
- An approved action does what it says, with your permissions. Read the card before you approve.
- Builds aren't code-signed yet; get them only from this repository's releases or build them yourself.

## Supported versions

Fixes go into the latest release only.
