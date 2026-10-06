// Helpers for Settings → Tools (MCP): turning what the user types or pastes
// into server settings. Secret values stay out of the settings file; they go
// to the keychain on Save.

export interface McpServer {
  name: string;
  command: string;
  args: string[];
  env_keys: string[];
  trusted: boolean;
  enabled: boolean;
}

/** Variable values typed in Settings, by server, not saved yet. */
export type McpEnv = Record<string, Record<string, string>>;

/** Splits a command line into words; "double" or 'single' quotes keep spaces. */
export function splitCommand(line: string): string[] {
  const words: string[] = [];
  let word = "";
  let quote: string | null = null;
  let started = false;
  for (const c of line.trim()) {
    if (quote) {
      if (c === quote) quote = null;
      else word += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      started = true;
    } else if (/\s/.test(c)) {
      if (started || word) words.push(word);
      word = "";
      started = false;
    } else {
      word += c;
    }
  }
  if (started || word) words.push(word);
  return words;
}

/** Joins words back into a line, quoting those with spaces. */
export function joinCommand(words: string[]): string {
  return words.map((w) => (w === "" || /\s|"/.test(w) ? `'${w}'` : w)).join(" ");
}

/** "NAME=value" lines. A line with just a name keeps its saved value. */
export function parseEnv(text: string): { keys: string[]; values: Record<string, string> } {
  const keys: string[] = [];
  const values: Record<string, string> = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    const key = (eq < 0 ? line : line.slice(0, eq)).trim();
    if (!key) continue;
    if (!keys.includes(key)) keys.push(key);
    const value = eq < 0 ? "" : line.slice(eq + 1).trim();
    if (value) values[key] = value;
  }
  return { keys, values };
}

/** Turns a name into one Waddle accepts: letters, digits, - and _, up to 24. */
export function cleanName(name: string): string {
  return name
    .trim()
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 24);
}

/**
 * Reads the JSON block many MCP servers document:
 * { "mcpServers": { "name": { "command": "npx", "args": [...], "env": { "KEY": "value" } } } }
 * (the outer braces and the "mcpServers" wrapper are optional).
 */
export function importJson(text: string): { servers: McpServer[]; env: McpEnv } {
  let data: unknown;
  const trimmed = text.trim();
  try {
    data = JSON.parse(trimmed);
  } catch {
    // People often copy just the inside: "name": { ... }
    data = JSON.parse(`{${trimmed.replace(/,\s*$/, "")}}`);
  }
  if (!data || typeof data !== "object") throw new Error("that isn't a JSON object");
  const obj = data as Record<string, unknown>;
  const block = (obj.mcpServers ?? obj.servers ?? obj) as Record<string, unknown>;
  const servers: McpServer[] = [];
  const env: McpEnv = {};
  for (const [rawName, value] of Object.entries(block)) {
    if (!value || typeof value !== "object") continue;
    const v = value as { command?: unknown; args?: unknown; env?: unknown; url?: unknown };
    if (typeof v.command !== "string") {
      if (typeof v.url === "string") throw new Error(`"${rawName}" is a remote server (a URL). Waddle runs local servers only for now.`);
      continue;
    }
    const name = cleanName(rawName);
    if (!name) continue;
    const args = Array.isArray(v.args) ? v.args.filter((a): a is string => typeof a === "string") : [];
    const values: Record<string, string> = {};
    if (v.env && typeof v.env === "object") {
      for (const [k, val] of Object.entries(v.env as Record<string, unknown>)) {
        if (typeof val === "string" || typeof val === "number") values[k] = String(val);
      }
    }
    servers.push({ name, command: v.command, args, env_keys: Object.keys(values), trusted: false, enabled: true });
    if (Object.keys(values).length) env[name] = values;
  }
  if (!servers.length) throw new Error("no servers with a command found");
  return { servers, env };
}
