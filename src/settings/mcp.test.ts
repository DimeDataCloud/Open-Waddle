import { describe, expect, it } from "vitest";
import { cleanName, importJson, joinCommand, parseEnv, splitCommand } from "./mcp";

describe("splitCommand", () => {
  it("splits words and keeps quoted spaces", () => {
    expect(splitCommand("npx -y @modelcontextprotocol/server-filesystem C:\\Users\\me\\Documents")).toEqual([
      "npx",
      "-y",
      "@modelcontextprotocol/server-filesystem",
      "C:\\Users\\me\\Documents",
    ]);
    expect(splitCommand(`uvx  mcp-server-git --repository "C:\\My Code\\app" ''`)).toEqual(["uvx", "mcp-server-git", "--repository", "C:\\My Code\\app", ""]);
    expect(splitCommand("   ")).toEqual([]);
  });

  it("round-trips through joinCommand", () => {
    const words = ["node", "C:\\My Tools\\server.js", "--name", "x"];
    expect(splitCommand(joinCommand(words))).toEqual(words);
  });
});

describe("parseEnv", () => {
  it("reads NAME=value lines, keeping names without values", () => {
    expect(parseEnv("GITHUB_TOKEN=ghp_abc=def\n# comment\n\nKEEP_ME\n")).toEqual({ keys: ["GITHUB_TOKEN", "KEEP_ME"], values: { GITHUB_TOKEN: "ghp_abc=def" } });
  });
});

describe("importJson", () => {
  it("reads the usual mcpServers block, with env values kept apart", () => {
    const { servers, env } = importJson(`{
      "mcpServers": {
        "GitHub Server": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_x" } },
        "time": { "command": "uvx", "args": ["mcp-server-time"] }
      }
    }`);
    expect(servers).toEqual([
      { name: "GitHub-Server", command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env_keys: ["GITHUB_PERSONAL_ACCESS_TOKEN"], trusted: false, enabled: true },
      { name: "time", command: "uvx", args: ["mcp-server-time"], env_keys: [], trusted: false, enabled: true },
    ]);
    expect(env).toEqual({ "GitHub-Server": { GITHUB_PERSONAL_ACCESS_TOKEN: "ghp_x" } });
  });

  it("accepts just the inside, and says what's wrong", () => {
    expect(importJson(`"memory": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-memory"] },`).servers[0].name).toBe("memory");
    expect(() => importJson(`{ "mcpServers": { "remote": { "url": "https://x.example/mcp" } } }`)).toThrow(/remote server/);
    expect(() => importJson(`{ "mcpServers": {} }`)).toThrow(/no servers/);
    expect(() => importJson("not json")).toThrow();
  });
});

describe("cleanName", () => {
  it("makes names Waddle accepts", () => {
    expect(cleanName(" My Notion! ")).toBe("My-Notion");
    expect(cleanName("x".repeat(30))).toHaveLength(24);
  });
});
