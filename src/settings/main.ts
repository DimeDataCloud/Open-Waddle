import { invoke } from "@tauri-apps/api/core";
import { cleanName, importJson, joinCommand, parseEnv, splitCommand, type McpEnv, type McpServer } from "./mcp";
import { Tabs } from "./tabs";

interface Settings {
  provider: "openai_compat" | "ollama" | "mock";
  base_url: string;
  model: string;
  fast_model: string;
  coord_mode: "auto" | "pixels" | "norm1000";
  tier2_mode: "countdown" | "ask";
  tier2_countdown_ms: number;
  max_steps: number;
  task_timeout_secs: number;
  command_timeout_secs: number;
  workspace_dir: string | null;
  wander: boolean;
  follow_monitors: boolean;
  record_traces: boolean;
  no_training: boolean;
  google_client_id: string;
  working_hours: [number, number];
  send_undo_secs: number;
  monthly_budget: number;
  meeting_nudges: boolean;
  mail_nudges: boolean;
  morning_brief: boolean;
  muted_senders: string[];
  autostart: boolean;
  read_user_folders: boolean;
  read_folders: string[];
  write_folders: string[];
  self_source_dir: string | null;
  max_delegation_depth: number;
  character: { id: string; color: string };
  ollama: { num_thread: number | null; keep_alive: string; num_ctx: number };
  voice: { backend: "system" | "whisper_api" | "off"; base_url: string; model: string; language: string | null };
  mcp_servers: McpServer[];
  first_run_done: boolean;
  voice_out: { enabled: boolean; replies: boolean; nudges: boolean; voice: string; rate: number; talk_mode: boolean };
}

interface SettingsView {
  settings: Settings;
  has_api_key: boolean;
  has_stt_key: boolean;
  workspace: string;
  demo: boolean;
  key_storage: string | null;
}

interface AuditRecord {
  id: number;
  ts_ms: number;
  task_id: string;
  kind: string;
  tool: string | null;
  tier: number | null;
  decision: string | null;
  detail: string | null;
}

const PRESETS: Record<string, Partial<Settings>> = {
  openrouter: { provider: "openai_compat", base_url: "https://openrouter.ai/api/v1", model: "openai/gpt-6-luna", fast_model: "google/gemini-2.5-flash-lite" },
  ollama: { provider: "ollama", base_url: "http://localhost:11434", model: "qwen3.5:4b", fast_model: "" },
  lmstudio: { provider: "openai_compat", base_url: "http://localhost:1234/v1", model: "qwen3.5-4b", fast_model: "" },
  foundry: { provider: "openai_compat", base_url: "http://localhost:5273/v1", model: "phi-4-mini", fast_model: "" },
  mock: { provider: "mock", base_url: "", model: "demo", fast_model: "" },
};

const $ = <T extends HTMLElement = HTMLInputElement>(id: string) => document.getElementById(id) as T;
let current: Settings;

function presetFor(s: Settings): string {
  if (s.provider === "mock") return "mock";
  if (s.provider === "ollama") return "ollama";
  if (s.base_url.includes("openrouter.ai")) return "openrouter";
  if (s.base_url.includes(":1234")) return "lmstudio";
  if (s.base_url.includes(":5273")) return "foundry";
  return "custom";
}

function syncVisibility(): void {
  const preset = $<HTMLSelectElement>("preset").value;
  // Demo mode needs no endpoint, model or key; hidden fields mustn't block Save.
  const demo = preset === "mock";
  $("brain-opts").classList.toggle("hidden", demo);
  $("base_url").required = !demo;
  $("model").required = !demo;
  $("ollama-opts").classList.toggle("hidden", preset !== "ollama");
  $("whisper-opts").classList.toggle("hidden", $<HTMLSelectElement>("voice_backend").value !== "whisper_api");
  $("vo-opts").classList.toggle("hidden", !$("vo_enabled").checked);
  $("vo_rate_label").textContent = `${Number($("vo_rate").value).toFixed(1)}×`;
}

/** Makes sure the saved voice is in the list, even before (or without) the system's list. */
function pickVoice(name: string): void {
  const select = $<HTMLSelectElement>("vo_voice");
  if (name && ![...select.options].some((o) => o.value === name)) select.add(new Option(name, name));
  select.value = name;
}

async function loadVoices(): Promise<void> {
  try {
    const names = await invoke<string[]>("speech_voices");
    const select = $<HTMLSelectElement>("vo_voice");
    const keep = select.value;
    for (const n of names) if (![...select.options].some((o) => o.value === n)) select.add(new Option(n, n));
    select.value = keep;
  } catch {
    // The default voice still works.
  }
}

// ---------- Tools (MCP) ----------

interface McpStatus {
  name: string;
  tools: string[];
  problem: string | null;
  saved_env: string[];
}

/** The servers as edited here; saved with the rest of the form. */
let mcpServers: McpServer[] = [];
/** Values typed for servers' variables, saved to the keychain with the form. */
let mcpEnv: McpEnv = {};
let mcpStatus: McpStatus[] = [];

function mcpLabel(s: McpServer): string {
  return joinCommand([s.command, ...s.args]);
}

function renderMcp(): void {
  const list = $("mcp-list");
  if (!mcpServers.length) {
    const li = document.createElement("li");
    li.className = "muted";
    li.textContent = "No servers yet.";
    list.replaceChildren(li);
    return;
  }
  list.replaceChildren(
    ...mcpServers.map((srv, i) => {
      const li = document.createElement("li");
      const info = document.createElement("div");
      info.className = "mcp-server";
      const name = document.createElement("strong");
      name.textContent = srv.name;
      const cmd = document.createElement("code");
      cmd.textContent = mcpLabel(srv);
      info.append(name, cmd);
      const st = mcpStatus.find((x) => x.name === srv.name);
      const pending = Object.keys(mcpEnv[srv.name] ?? {});
      if (srv.env_keys.length) {
        const env = document.createElement("div");
        env.className = "tools";
        env.textContent = `Needs ${srv.env_keys
          .map((k) => `${k}${st?.saved_env.includes(k) || pending.includes(k) ? " ✓" : " (no value yet)"}`)
          .join(", ")}`;
        info.append(env);
      }
      const line = document.createElement("div");
      if (st?.problem) {
        line.className = "problem";
        line.textContent = `Couldn't start it: ${st.problem}`;
      } else {
        line.className = "tools";
        line.textContent = st?.tools.length ? `${st.tools.length} tools: ${st.tools.slice(0, 8).join(", ")}${st.tools.length > 8 ? "…" : ""}` : "Tools are listed after Save.";
      }
      info.append(line);
      const actions = document.createElement("div");
      actions.className = "mcp-actions";
      const toggle = (label: string, key: "enabled" | "trusted") => {
        const l = document.createElement("label");
        l.className = "check";
        const box = document.createElement("input");
        box.type = "checkbox";
        box.checked = srv[key];
        box.onchange = () => (srv[key] = box.checked);
        l.append(box, ` ${label}`);
        return l;
      };
      const test = document.createElement("button");
      test.type = "button";
      test.className = "link";
      test.textContent = "Test";
      test.onclick = () => void testMcp(srv, mcpEnv[srv.name] ?? {}, line);
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "link";
      remove.textContent = "Remove";
      remove.onclick = () => {
        mcpServers.splice(i, 1);
        delete mcpEnv[srv.name];
        renderMcp();
      };
      actions.append(toggle("On", "enabled"), toggle("Trusted", "trusted"), test, remove);
      li.append(info, actions);
      return li;
    }),
  );
}

async function testMcp(srv: McpServer, env: Record<string, string>, out: HTMLElement): Promise<void> {
  out.className = "tools";
  out.textContent = "Starting it…";
  try {
    const tools = await invoke<string[]>("mcp_test", { server: srv, env });
    out.textContent = tools.length ? `Works: ${tools.length} tools: ${tools.join(", ")}` : "It started but offers no tools.";
  } catch (err) {
    out.className = "problem";
    out.textContent = `Couldn't start it: ${err}`;
  }
}

async function loadMcp(): Promise<void> {
  try {
    mcpStatus = await invoke<McpStatus[]>("mcp_status");
  } catch {
    mcpStatus = [];
  }
  renderMcp();
}

function draftServer(): { server: McpServer; values: Record<string, string> } | null {
  const words = splitCommand($("mcp_command").value);
  const name = cleanName($("mcp_name").value) || cleanName(words[words.length - 1]?.split("/").pop() ?? "");
  if (!words.length || !name) {
    $("mcp-status").textContent = "Give it a name and a command.";
    return null;
  }
  const env = parseEnv($<HTMLTextAreaElement>("mcp_env").value);
  return {
    server: { name, command: words[0], args: words.slice(1), env_keys: env.keys, trusted: $("mcp_trusted").checked, enabled: true },
    values: env.values,
  };
}

function addServers(servers: McpServer[], env: McpEnv): void {
  for (const srv of servers) {
    let name = srv.name;
    for (let n = 2; mcpServers.some((x) => x.name.toLowerCase() === name.toLowerCase()); n++) name = `${srv.name.slice(0, 21)}-${n}`;
    mcpServers.push({ ...srv, name });
    if (env[srv.name]) mcpEnv[name] = { ...env[srv.name] };
  }
  renderMcp();
}

$("mcp-test").addEventListener("click", () => {
  const d = draftServer();
  if (d) void testMcp(d.server, d.values, $("mcp-status"));
});

$("mcp-add-btn").addEventListener("click", () => {
  const d = draftServer();
  if (!d) return;
  addServers([d.server], { [d.server.name]: d.values });
  for (const id of ["mcp_name", "mcp_command"]) $(id).value = "";
  $<HTMLTextAreaElement>("mcp_env").value = "";
  $("mcp_trusted").checked = false;
  $("mcp-status").textContent = "Added. Press Save to start using it.";
});

$("mcp-import").addEventListener("click", () => {
  try {
    const { servers, env } = importJson($<HTMLTextAreaElement>("mcp_json").value);
    addServers(servers, env);
    $<HTMLTextAreaElement>("mcp_json").value = "";
    $("mcp-status").textContent = `Imported ${servers.map((s) => s.name).join(", ")}. Press Save to start using ${servers.length > 1 ? "them" : "it"}.`;
  } catch (err) {
    $("mcp-status").textContent = `Couldn't read that: ${err instanceof Error ? err.message : err}`;
  }
});

function fill(view: SettingsView): void {
  const s = (current = view.settings);
  mcpServers = s.mcp_servers.map((x) => ({ ...x, args: [...x.args], env_keys: [...x.env_keys] }));
  mcpEnv = {};
  void loadMcp();
  // Servers saved just now list their tools in the background.
  if (mcpServers.length) for (const ms of [2500, 8000]) window.setTimeout(() => void loadMcp(), ms);
  $<HTMLSelectElement>("preset").value = presetFor(s);
  $("base_url").value = s.base_url;
  $("model").value = s.model;
  $("fast_model").value = s.fast_model;
  $("api_key").value = "";
  $("api_key").placeholder = view.has_api_key ? "saved (leave blank to keep)" : "paste your key";
  $<HTMLSelectElement>("coord_mode").value = s.coord_mode;
  $("num_thread").value = s.ollama.num_thread?.toString() ?? "";
  $("keep_alive").value = s.ollama.keep_alive;
  $("num_ctx").value = String(s.ollama.num_ctx);
  $<HTMLSelectElement>("tier2_mode").value = s.tier2_mode;
  $("tier2_countdown_ms").value = String(s.tier2_countdown_ms);
  $("max_steps").value = String(s.max_steps);
  $("command_timeout_secs").value = String(s.command_timeout_secs);
  $("workspace_dir").value = s.workspace_dir ?? "";
  $("workspace_dir").placeholder = view.workspace;
  $<HTMLSelectElement>("voice_backend").value = s.voice.backend;
  $("voice_base_url").value = s.voice.base_url;
  $("voice_model").value = s.voice.model;
  $("voice_language").value = s.voice.language ?? "";
  $("vo_enabled").checked = s.voice_out.enabled;
  $("vo_replies").checked = s.voice_out.replies;
  $("vo_nudges").checked = s.voice_out.nudges;
  pickVoice(s.voice_out.voice);
  $("vo_rate").value = String(s.voice_out.rate);
  $("vo_talk").checked = s.voice_out.talk_mode;
  $("stt_key").value = "";
  $("stt_key").placeholder = view.has_stt_key ? "saved (leave blank to keep)" : "e.g. a free Groq key";
  $("self_source_dir").value = s.self_source_dir ?? "";
  $("max_delegation_depth").value = String(s.max_delegation_depth);
  $("color").value = s.character.color;
  $("wander").checked = s.wander;
  $("follow_monitors").checked = s.follow_monitors;
  $("record_traces").checked = s.record_traces;
  $("no_training").checked = s.no_training;
  $("google_client_id").value = s.google_client_id;
  $("work_start").value = String(s.working_hours[0]);
  $("work_end").value = String(s.working_hours[1]);
  $("send_undo_secs").value = String(s.send_undo_secs);
  $("monthly_budget").value = String(s.monthly_budget);
  $("meeting_nudges").checked = s.meeting_nudges;
  $("mail_nudges").checked = s.mail_nudges;
  $("morning_brief").checked = s.morning_brief;
  $<HTMLTextAreaElement>("muted_senders").value = s.muted_senders.join("\n");
  $("autostart").checked = s.autostart;
  $("read_user_folders").checked = s.read_user_folders;
  $<HTMLTextAreaElement>("read_folders").value = s.read_folders.join("\n");
  $<HTMLTextAreaElement>("write_folders").value = s.write_folders.join("\n");
  $("mode").textContent = view.demo
    ? "Demo mode: add an API key (or pick a local model) to make Waddle useful."
    : `Using ${s.model}. Workspace: ${view.workspace}`;
  syncVisibility();
}

const lines = (id: string) =>
  $<HTMLTextAreaElement>(id)
    .value.split("\n")
    .map((x) => x.trim())
    .filter(Boolean);

function collect(): Settings {
  const num = (id: string, fallback: number) => {
    const v = Number($(id).value);
    return Number.isFinite(v) && $(id).value !== "" ? v : fallback;
  };
  const threads = $("num_thread").value.trim();
  return {
    ...current,
    provider: PRESETS[$<HTMLSelectElement>("preset").value]?.provider ?? "openai_compat",
    base_url: $("base_url").value.trim(),
    model: $("model").value.trim(),
    fast_model: $("fast_model").value.trim(),
    coord_mode: $<HTMLSelectElement>("coord_mode").value as Settings["coord_mode"],
    tier2_mode: $<HTMLSelectElement>("tier2_mode").value as Settings["tier2_mode"],
    tier2_countdown_ms: num("tier2_countdown_ms", 2000),
    max_steps: num("max_steps", 20),
    command_timeout_secs: num("command_timeout_secs", 60),
    workspace_dir: $("workspace_dir").value.trim() || null,
    wander: $("wander").checked,
    follow_monitors: $("follow_monitors").checked,
    record_traces: $("record_traces").checked,
    no_training: $("no_training").checked,
    google_client_id: $("google_client_id").value.trim(),
    working_hours: [num("work_start", 9), num("work_end", 17)],
    send_undo_secs: num("send_undo_secs", 10),
    monthly_budget: Math.max(0, num("monthly_budget", 5)),
    meeting_nudges: $("meeting_nudges").checked,
    mail_nudges: $("mail_nudges").checked,
    morning_brief: $("morning_brief").checked,
    muted_senders: $<HTMLTextAreaElement>("muted_senders")
      .value.split("\n")
      .map((x) => x.trim())
      .filter(Boolean),
    autostart: $("autostart").checked,
    read_user_folders: $("read_user_folders").checked,
    read_folders: lines("read_folders"),
    write_folders: lines("write_folders"),
    self_source_dir: $("self_source_dir").value.trim() || null,
    max_delegation_depth: num("max_delegation_depth", 2),
    character: { ...current.character, color: $("color").value },
    ollama: { num_thread: threads ? Number(threads) : null, keep_alive: $("keep_alive").value.trim() || "30s", num_ctx: num("num_ctx", 8192) },
    voice: {
      backend: $<HTMLSelectElement>("voice_backend").value as Settings["voice"]["backend"],
      base_url: $("voice_base_url").value.trim(),
      model: $("voice_model").value.trim(),
      language: $("voice_language").value.trim() || null,
    },
    mcp_servers: mcpServers,
    voice_out: {
      enabled: $("vo_enabled").checked,
      replies: $("vo_replies").checked,
      nudges: $("vo_nudges").checked,
      voice: $<HTMLSelectElement>("vo_voice").value,
      rate: num("vo_rate", 1),
      talk_mode: $("vo_talk").checked,
    },
  };
}

interface SpendLine {
  calls: number;
  cost: number;
  tokens_in: number;
  tokens_out: number;
}

interface Spending {
  today: number;
  month: number;
  budget: number;
  paused: boolean;
  by_purpose: Record<string, SpendLine>;
  days: { day: string; cost: number }[];
}

/** Pennies matter here: a day of chat can cost a tenth of a cent. */
const dollars = (x: number) =>
  x >= 1 ? `$${x.toFixed(2)}` : x >= 0.01 ? `$${x.toFixed(3)}` : x >= 0.0001 ? `$${x.toFixed(4)}` : x > 0 ? "<$0.0001" : "$0.00";

async function loadSpending(): Promise<void> {
  let s: Spending;
  try {
    s = await invoke<Spending>("spending");
  } catch {
    return;
  }
  const of = s.budget > 0 ? ` of ${dollars(s.budget)}` : "";
  $("spend-summary").textContent = s.paused
    ? `This month ${dollars(s.month)}${of}: the budget is used up, so paid model calls are paused until the 1st.`
    : `Today ${dollars(s.today)} · this month ${dollars(s.month)}${of}`;
  const max = Math.max(...s.days.map((d) => d.cost), 1e-9);
  $("spend-bars").replaceChildren(
    ...s.days.map((d) => {
      const bar = document.createElement("div");
      bar.style.height = `${Math.max(2, Math.round((d.cost / max) * 100))}%`;
      bar.title = `${d.day}: ${dollars(d.cost)}`;
      if (d.cost === 0) bar.classList.add("zero");
      return bar;
    }),
  );
  const rows = Object.entries(s.by_purpose).sort((a, b) => b[1].cost - a[1].cost);
  $("spend-purposes").replaceChildren(
    ...rows.map(([name, l]) => {
      const li = document.createElement("li");
      li.textContent = `${name[0].toUpperCase()}${name.slice(1)}: ${dollars(l.cost)} (${l.calls} call${l.calls === 1 ? "" : "s"})`;
      return li;
    }),
  );
}

async function loadSkills(): Promise<void> {
  const skills = await invoke<{ name: string; body: string }[]>("skills_list");
  const list = $("skills");
  if (!skills.length) {
    const li = document.createElement("li");
    li.className = "muted";
    li.textContent = "None yet. Waddle saves skills as it learns (you approve each one).";
    list.replaceChildren(li);
    return;
  }
  list.replaceChildren(
    ...skills.map((sk) => {
      const li = document.createElement("li");
      const text = document.createElement("div");
      const name = document.createElement("strong");
      name.textContent = sk.name;
      const body = document.createElement("pre");
      body.textContent = sk.body;
      text.append(name, body);
      const forget = document.createElement("button");
      forget.type = "button";
      forget.className = "link";
      forget.textContent = "Forget";
      forget.onclick = async () => {
        await invoke("skill_forget", { name: sk.name });
        void loadSkills();
      };
      li.append(text, forget);
      return li;
    }),
  );
}

interface RoutineView {
  id: string;
  goal: string;
  schedule: string;
  next: string;
  paused: boolean;
  last_result: string | null;
}

function renderRoutines(list: RoutineView[]): void {
  const ul = $("routines");
  if (!list.length) {
    const li = document.createElement("li");
    li.className = "muted";
    li.textContent = "No routines yet.";
    ul.replaceChildren(li);
    return;
  }
  ul.replaceChildren(
    ...list.map((r) => {
      const li = document.createElement("li");
      const text = document.createElement("div");
      text.className = "mcp-server";
      const goal = document.createElement("strong");
      goal.textContent = r.goal;
      const when = document.createElement("div");
      when.className = "tools";
      when.textContent = `${r.schedule}${r.paused ? " (paused)" : r.next === "finished" ? " (finished)" : `, next ${r.next}`}`;
      text.append(goal, when);
      if (r.last_result) {
        const last = document.createElement("div");
        last.className = r.last_result.startsWith("Done") ? "tools" : "problem";
        last.textContent = `Last time: ${r.last_result}`;
        text.append(last);
      }
      const actions = document.createElement("div");
      actions.className = "mcp-actions";
      const pause = document.createElement("button");
      pause.type = "button";
      pause.className = "link";
      pause.textContent = r.paused ? "Resume" : "Pause";
      pause.onclick = async () => renderRoutines(await invoke<RoutineView[]>("routine_pause", { id: r.id, paused: !r.paused }));
      const del = document.createElement("button");
      del.type = "button";
      del.className = "link";
      del.textContent = "Delete";
      del.onclick = async () => renderRoutines(await invoke<RoutineView[]>("routine_delete", { id: r.id }));
      actions.append(pause, del);
      li.append(text, actions);
      return li;
    }),
  );
}

async function loadRoutines(): Promise<void> {
  try {
    renderRoutines(await invoke<RoutineView[]>("routines_list"));
  } catch {
    // Shown empty.
  }
}

$("routine-add").addEventListener("click", async () => {
  const status = $("routine-status");
  const goal = $("routine_goal").value.trim();
  if (!goal) {
    status.textContent = "Say what the routine should do.";
    return;
  }
  try {
    renderRoutines(await invoke<RoutineView[]>("routine_add", { goal, time: $("routine_time").value, days: [$<HTMLSelectElement>("routine_days").value] }));
    $("routine_goal").value = "";
    status.textContent = "Added.";
  } catch (err) {
    status.textContent = String(err);
  }
});

async function loadFacts(): Promise<void> {
  const facts = await invoke<{ id: string; text: string }[]>("facts_list");
  const list = $("facts");
  if (!facts.length) {
    const li = document.createElement("li");
    li.className = "muted";
    li.textContent = "Nothing yet.";
    list.replaceChildren(li);
    return;
  }
  list.replaceChildren(
    ...facts.map((f) => {
      const li = document.createElement("li");
      const text = document.createElement("span");
      text.textContent = f.text;
      const forget = document.createElement("button");
      forget.type = "button";
      forget.className = "link";
      forget.textContent = "Forget";
      forget.onclick = async () => {
        await invoke("fact_forget", { id: f.id });
        void loadFacts();
      };
      li.append(text, forget);
      return li;
    }),
  );
}

async function addFact(): Promise<void> {
  const input = $("fact-text");
  const text = input.value.trim();
  if (!text) return;
  try {
    await invoke("fact_add", { text });
    input.value = "";
  } catch (err) {
    $("status").textContent = String(err);
  }
  void loadFacts();
}

$("fact-add").addEventListener("click", () => void addFact());
$("fact-text").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    void addFact();
  }
});

interface GoogleStatus {
  client_id: string;
  has_secret: boolean;
  connected: boolean;
  email: string | null;
  error: string | null;
  expired: boolean;
}

function showGoogle(g: GoogleStatus): void {
  $("google-connect").classList.toggle("hidden", g.connected && !g.expired);
  $("google-disconnect").classList.toggle("hidden", !g.connected);
  $("google_client_secret").placeholder = g.has_secret ? "saved (leave blank to keep)" : "from the same page as the client ID";
  $("google-status").textContent = g.expired
    ? "Sign-in expired: Google signed Waddle out. Press Connect to sign in again."
    : g.connected
    ? g.email
      ? `Connected as ${g.email}.`
      : `Connected, but Google didn't answer: ${g.error ?? "unknown error"}`
    : "Not connected. Without it, Waddle uses Gmail and Calendar in Chrome on screen.";
}

async function loadGoogle(): Promise<void> {
  try {
    showGoogle(await invoke<GoogleStatus>("google_status"));
  } catch (err) {
    $("google-status").textContent = String(err);
  }
}

$("google-connect").addEventListener("click", async () => {
  const button = $<HTMLButtonElement>("google-connect");
  if (button.dataset.waiting) {
    await invoke("google_cancel");
    return;
  }
  button.dataset.waiting = "1";
  button.textContent = "Cancel";
  $("google-status").textContent = "Finish signing in in your browser…";
  try {
    const secret = $("google_client_secret").value.trim();
    showGoogle(await invoke<GoogleStatus>("google_connect", { clientId: $("google_client_id").value.trim(), clientSecret: secret || null }));
    $("google_client_secret").value = "";
  } catch (err) {
    $("google-status").textContent = `Couldn't connect: ${err}`;
  } finally {
    delete button.dataset.waiting;
    button.textContent = "Connect";
  }
});

$("google-disconnect").addEventListener("click", async () => {
  showGoogle(await invoke<GoogleStatus>("google_disconnect"));
});

$("style-save").addEventListener("click", async () => {
  await invoke("style_set", { text: $<HTMLTextAreaElement>("style-note").value });
  $("style-save").textContent = "Saved";
  setTimeout(() => ($("style-save").textContent = "Save note"), 1500);
});

interface BrowserStatus {
  connected: boolean;
  version: string | null;
  folder: string | null;
}

function showBrowser(b: BrowserStatus): void {
  $("browser-status").textContent = b.connected
    ? `Connected (extension ${b.version ?? "?"}).`
    : b.folder
      ? "Not connected. Is Chrome open with the extension loaded?"
      : "Not set up yet.";
  if (b.folder) $("browser-folder").textContent = b.folder;
  $("browser-steps").classList.toggle("hidden", b.connected || !b.folder);
}

async function loadBrowser(): Promise<void> {
  try {
    showBrowser(await invoke<BrowserStatus>("browser_status"));
  } catch (err) {
    $("browser-status").textContent = String(err);
  }
}

$("browser-setup").addEventListener("click", async () => {
  try {
    showBrowser(await invoke<BrowserStatus>("browser_setup"));
  } catch (err) {
    $("browser-status").textContent = `Couldn't set up: ${err}`;
  }
});

async function loadStyle(): Promise<void> {
  const note = $<HTMLTextAreaElement>("style-note");
  if (document.activeElement !== note) note.value = await invoke<string>("style_get");
}

async function loadAudit(): Promise<void> {
  const rows = await invoke<AuditRecord[]>("audit_recent", { limit: 50 });
  const body = $("audit");
  body.replaceChildren(
    ...rows.map((r) => {
      const tr = document.createElement("tr");
      const cells = [
        new Date(r.ts_ms).toLocaleTimeString(),
        r.tool ? `${r.kind}: ${r.tool}` : r.kind,
        r.tier?.toString() ?? "",
        r.decision ?? "",
        r.detail ?? "",
      ];
      cells.forEach((text, i) => {
        const td = document.createElement("td");
        td.textContent = text;
        if (i === 4) td.className = "detail";
        tr.appendChild(td);
      });
      return tr;
    }),
  );
}

$<HTMLSelectElement>("preset").addEventListener("change", () => {
  const p = PRESETS[$<HTMLSelectElement>("preset").value];
  if (p) {
    $("base_url").value = p.base_url ?? "";
    $("model").value = p.model ?? "";
    $("fast_model").value = p.fast_model ?? "";
  }
  syncVisibility();
});
$<HTMLSelectElement>("voice_backend").addEventListener("change", syncVisibility);
$("vo_enabled").addEventListener("change", syncVisibility);
$("vo_rate").addEventListener("input", syncVisibility);
$("vo-test").addEventListener("click", async () => {
  const status = $("vo-status");
  status.textContent = "";
  try {
    await invoke("speech_test", { voice: $<HTMLSelectElement>("vo_voice").value, rate: Number($("vo_rate").value) });
    status.textContent = "Listen…";
  } catch (err) {
    status.textContent = String(err);
  }
});
$("open-ws").addEventListener("click", () => void invoke("open_workspace"));
$("verify").addEventListener("click", async () => {
  const r = await invoke<{ ok: boolean; entries: number; first_bad_id: number | null }>("audit_verify");
  $("verify-result").textContent = r.ok
    ? `Log intact: ${r.entries} entries verified.`
    : `Tampering detected at entry ${r.first_bad_id}.`;
});

interface Check {
  name: string;
  status: "pass" | "warn" | "fail" | "skip";
  detail: string;
  millis: number;
}

const MARKS: Record<Check["status"], string> = { pass: "OK", warn: "!", fail: "FAIL", skip: "–" };

$("selftest").addEventListener("click", async () => {
  const button = $<HTMLButtonElement>("selftest");
  const result = $("selftest-result");
  button.disabled = true;
  result.textContent = "Running… (the model check can take a little while)";
  $("checks").replaceChildren();
  try {
    const r = await invoke<{ checks: Check[]; report: string }>("run_self_test");
    $("checks").replaceChildren(
      ...r.checks.map((c) => {
        const li = document.createElement("li");
        li.className = c.status;
        for (const [cls, text] of [["mark", MARKS[c.status]], ["name", c.name], ["detail", c.detail]]) {
          const span = document.createElement("span");
          span.className = cls;
          span.textContent = text;
          li.appendChild(span);
        }
        return li;
      }),
    );
    const bad = r.checks.filter((c) => c.status === "fail" || c.status === "warn").length;
    result.textContent = `${bad ? `${bad} item(s) need attention. ` : "Everything checks out. "}Report saved to ${r.report}`;
  } catch (err) {
    result.textContent = `Self-test couldn't run: ${err}`;
  } finally {
    button.disabled = false;
  }
});

const savedTab = (() => {
  try {
    return localStorage.getItem("settings-tab") ?? "brain";
  } catch {
    return "brain";
  }
})();
const tabs = new Tabs($("tabs"), $<HTMLInputElement>("search"), $("no-match"), document, document.querySelector<HTMLElement>(".actions"), savedTab);

// A required field on another tab would block Save without showing why: open its tab first.
document.querySelector<HTMLButtonElement>("button[type=submit]")!.addEventListener("click", (e) => {
  const form = $<HTMLFormElement>("form");
  if (form.checkValidity()) return;
  e.preventDefault();
  const bad = form.querySelector(":invalid");
  if (bad) tabs.reveal(bad);
  form.reportValidity();
});

$<HTMLFormElement>("form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const status = $("status");
  status.textContent = "Saving…";
  try {
    const apiKey = $("api_key").value.trim();
    const sttKey = $("stt_key").value.trim();
    const view = await invoke<SettingsView>("save_settings", {
      settings: collect(),
      apiKey: apiKey ? apiKey : null,
      sttKey: sttKey ? sttKey : null,
      mcpEnv,
    });
    fill(view);
    status.textContent = view.key_storage === "file" ? "Saved (keychain unavailable: key stored in a private file)." : "Saved.";
  } catch (err) {
    status.textContent = `Couldn't save: ${err}`;
  }
});

async function loadTraces(): Promise<void> {
  try {
    const t = await invoke<{ folder: string; total: number; good: number; bad: number }>("traces_summary");
    $("traces").textContent = t.total
      ? `${t.total} saved task(s): ${t.good} 👍, ${t.bad} 👎. Folder: ${t.folder}`
      : "No saved tasks yet.";
  } catch {
    $("traces").textContent = "";
  }
}

$("forget-conversation").addEventListener("click", async () => {
  await invoke("clear_memory");
  $("forgot").textContent = "Forgotten.";
});

$("export-traces").addEventListener("click", async () => {
  try {
    $("traces").textContent = `Exported: ${await invoke<string>("export_traces")}. See docs/TRAINING.md for the next step.`;
  } catch (err) {
    $("traces").textContent = `Couldn't export: ${err}`;
  }
});

// ---------- first-run welcome ----------

let ollamaModel: string | null = null;

function showWelcome(on: boolean): void {
  document.body.classList.toggle("welcoming", on);
  $("welcome").classList.toggle("hidden", !on);
}

function welcomeChoice(): string {
  return document.querySelector<HTMLInputElement>('input[name="brain"]:checked')?.value ?? "openrouter";
}

for (const r of document.querySelectorAll<HTMLInputElement>('input[name="brain"]')) {
  r.addEventListener("change", () => {
    $("w-openrouter").classList.toggle("hidden", welcomeChoice() !== "openrouter");
    $("w-ollama").classList.toggle("hidden", welcomeChoice() !== "ollama");
  });
}

$("w-test").addEventListener("click", async () => {
  const status = $("w-test-status");
  status.textContent = "Asking the model…";
  try {
    status.textContent = await invoke<string>("test_key", { settings: { ...current, ...PRESETS.openrouter }, key: $("w_key").value.trim() || null });
  } catch (err) {
    status.textContent = String(err);
  }
});

$("w-detect").addEventListener("click", async () => {
  const status = $("w-ollama-status");
  status.textContent = "Looking…";
  try {
    const [models, pick] = await invoke<[string[], string | null]>("detect_ollama", { baseUrl: null });
    ollamaModel = pick;
    status.textContent = pick ? `Found Ollama with ${models.length} model${models.length === 1 ? "" : "s"}. I'll use ${pick}.` : "Ollama is running but has no models yet. Run: ollama pull qwen3.5:4b";
  } catch (err) {
    ollamaModel = null;
    status.textContent = String(err);
  }
});

async function finishWelcome(skip: boolean): Promise<void> {
  const status = $("w-status");
  let settings: Settings = { ...current, first_run_done: true };
  let apiKey: string | null = null;
  if (!skip) {
    settings = { ...settings, character: { ...current.character, color: $("w_color").value } };
    const choice = welcomeChoice();
    if (choice === "openrouter") {
      apiKey = $("w_key").value.trim() || null;
      if (!apiKey) {
        status.textContent = "Paste your OpenRouter key first, or pick another option.";
        return;
      }
      settings = { ...settings, ...PRESETS.openrouter };
    } else if (choice === "ollama") {
      if (!ollamaModel) {
        status.textContent = "Press “Look for Ollama” first (it needs to be running).";
        return;
      }
      settings = { ...settings, ...PRESETS.ollama, model: ollamaModel };
    }
  }
  status.textContent = "Saving…";
  try {
    const view = await invoke<SettingsView>("save_settings", { settings, apiKey, sttKey: null, mcpEnv: {} });
    fill(view);
    showWelcome(false);
  } catch (err) {
    status.textContent = `Couldn't save: ${err}`;
  }
}

$("w-done").addEventListener("click", () => void finishWelcome(false));
$("w-skip").addEventListener("click", () => void finishWelcome(true));

void invoke<SettingsView>("get_settings").then((view) => {
  fill(view);
  if (view.demo && !view.settings.first_run_done) {
    $("w_color").value = view.settings.character.color;
    showWelcome(true);
  }
});
void loadTraces();
void loadSpending();
void loadAudit();
void loadSkills();
void loadFacts();
void loadGoogle();
void loadStyle();
void loadBrowser();
void loadVoices();
void loadRoutines();
setInterval(() => {
  void loadSpending();
  void loadBrowser();
  void loadStyle();
  void loadAudit();
  void loadSkills();
  // Don't redraw the list under the user while they type a new fact.
  if (document.activeElement !== $("fact-text")) void loadFacts();
}, 5000);
