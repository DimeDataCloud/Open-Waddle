# MCP tools

Many apps and services publish an **MCP server**: a small program that offers tools to AI assistants. Examples are GitHub (issues, pull requests), Notion, Slack, databases, your notes, and the time in other cities. Waddle can use the tools of any MCP server you add.

## Adding a server

Settings → **Tools (MCP)** → **Add a server**. You can add a server in either of two ways:

- **Type it in**: a name (like `github`), the command that starts it, and any environment variables it needs as `NAME=value` lines.
- **Paste its JSON**: most servers' instructions show a block like the one below. Paste it into the JSON box and press **Import**.

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_…" }
    }
  }
}
```

Press **Test** to check that the server starts. Then press **Save**. Waddle lists the server's tools in the background, and they're offered to every task from then on.

Some example servers:

| Server | Command | Needs |
|---|---|---|
| Files in a folder | `npx -y @modelcontextprotocol/server-filesystem C:\Users\you\Notes` | Node.js |
| Memory (a knowledge graph) | `npx -y @modelcontextprotocol/server-memory` | Node.js |
| Time and time zones | `uvx mcp-server-time` | Python's `uv` |
| GitHub | `npx -y @modelcontextprotocol/server-github` | Node.js and `GITHUB_PERSONAL_ACCESS_TOKEN` |

Servers that run with `npx` need [Node.js](https://nodejs.org). Servers that run with `uvx` need [uv](https://docs.astral.sh/uv/). Only local servers (a command) are supported for now; remote servers (a URL) aren't.

## What Waddle allows

- **Approval.** A server you haven't marked **trusted** waits for your click before each tool runs. For tools that say they only read, there's a countdown you can cancel instead.
- **Trusted servers.** Their tools that say they only read run straight away. Tools that change things run after a countdown, and tools that say they may delete or overwrite wait for your click.
- **Answers are untrusted.** Whatever a tool returns is treated like a web page: Waddle reads it, but never follows instructions in it.
- **Waddle can't change your servers.** It can't add, change or remove servers, or read their keys.
- **Keys** you give a server (the `NAME=value` lines) are kept in Windows Credential Manager, not in the settings file.
- **Running.** A server starts the first time a task uses it and stops after 10 minutes without use. Each one runs as its own program, with no window.
- **Logs.** Every tool call is written to the activity log (Settings → Activity log).

## When something goes wrong

- **"Couldn't start it"** usually means the command isn't installed. For example, `npx` needs Node.js. Run the command in a terminal to see the error.
- **No tools listed after Save.** Press **Test** to see what the server says. A missing key is the most common cause.
- **A tool fails.** Waddle tells the model what the server said, and it tries something else or tells you.
