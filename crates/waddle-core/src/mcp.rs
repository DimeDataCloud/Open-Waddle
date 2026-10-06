//! MCP (Model Context Protocol) tools. The user adds servers in Settings
//! (a command Waddle starts, speaking JSON-RPC over stdin/stdout); their tools
//! are offered to the planner as `mcp_<server>_<tool>`.
//!
//! - Tool lists are cached on disk, so tasks can offer them without starting
//!   anything; a server starts on its first call and stops after 10 idle minutes.
//! - What a tool returns is untrusted text, like a web page.
//! - Tiers come from fixed rules (`safety::classify_mcp`): the server's own
//!   read-only hint only lowers the tier for servers the user marked trusted.
//! - The model can't add, change or remove servers.

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::llm::ToolSpec;
use crate::tools::ToolOutcome;

/// The protocol version Waddle asks for. Servers may answer with an older one they speak.
pub const PROTOCOL: &str = "2025-11-25";
/// Versions whose tools/list and tools/call Waddle understands.
const SPOKEN: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const START_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// A server nobody has used for this long is stopped (it starts again when needed).
pub const IDLE_STOP: Duration = Duration::from_secs(600);
/// A tool's answer is cut here.
const MAX_RESULT: usize = 16_000;
const MAX_DESCRIPTION: usize = 1_000;
/// Offered tool names: what OpenAI-style APIs accept.
const MAX_NAME: usize = 64;

/// One server, as the app starts it: settings plus the secret environment values.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub trusted: bool,
}

impl Launch {
    /// Changes when the server would start differently, so its cached tool list is redone.
    fn fingerprint(&self) -> String {
        let mut keys: Vec<&str> = self.env.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort();
        format!("{}\u{1f}{}\u{1f}{}", self.command, self.args.join("\u{1f}"), keys.join(","))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    /// The server says the tool only reads.
    #[serde(default)]
    pub read_only: bool,
    /// The server says the tool may destroy or overwrite things.
    #[serde(default)]
    pub destructive: bool,
}

impl McpTool {
    fn parse(v: &Value) -> Option<McpTool> {
        let ann = &v["annotations"];
        Some(McpTool {
            name: v["name"].as_str()?.to_string(),
            description: v["description"].as_str().unwrap_or("").chars().take(MAX_DESCRIPTION).collect(),
            input_schema: v["inputSchema"].clone(),
            read_only: ann["readOnlyHint"].as_bool().unwrap_or(false),
            // The spec's default for a tool that isn't read-only is "may be destructive";
            // only an explicit false counts as not.
            destructive: !ann["readOnlyHint"].as_bool().unwrap_or(false) && ann["destructiveHint"].as_bool().unwrap_or(true),
        })
    }
}

/// What the safety rules need to know about an offered tool.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub server: String,
    pub tool: McpTool,
    pub trusted: bool,
}

// ---------- one running server ----------

type Reply = Result<Value, String>;

struct Client {
    child: Mutex<Option<Child>>,
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    reader: tokio::task::JoinHandle<()>,
    /// The last lines the server wrote to stderr, for error messages.
    stderr: Arc<Mutex<Vec<String>>>,
}

/// On Windows, `npx` is really `npx.cmd`: find the file PATH would run.
#[cfg(windows)]
fn resolve(command: &str) -> PathBuf {
    let p = std::path::Path::new(command);
    if p.extension().is_some() || p.components().count() > 1 {
        return p.to_path_buf();
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{command}{}", ext.to_ascii_lowercase()));
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    p.to_path_buf()
}

#[cfg(not(windows))]
fn resolve(command: &str) -> PathBuf {
    PathBuf::from(command)
}

async fn write_line(stdin: &tokio::sync::Mutex<Option<ChildStdin>>, msg: &Value) -> anyhow::Result<()> {
    let mut guard = stdin.lock().await;
    let s = guard.as_mut().ok_or_else(|| anyhow!("the server has stopped"))?;
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    s.write_all(&line).await?;
    s.flush().await?;
    Ok(())
}

impl Client {
    async fn start(launch: &Launch, cancel: &CancellationToken) -> anyhow::Result<Client> {
        let mut cmd = tokio::process::Command::new(resolve(&launch.command));
        cmd.args(&launch.args)
            .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
        #[cfg(unix)]
        cmd.process_group(0); // Ctrl+C in a terminal running Waddle doesn't reach it
        let mut child = cmd.spawn().with_context(|| format!("couldn't start `{}`", launch.command))?;
        let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr_pipe = child.stderr.take();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>> = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let stderr: Arc<Mutex<Vec<String>>> = Arc::default();
        if let Some(pipe) = stderr_pipe {
            let (stderr, name) = (stderr.clone(), launch.name.clone());
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("mcp {name}: {line}");
                    let mut s = stderr.lock().unwrap();
                    s.push(line.chars().take(300).collect());
                    let excess = s.len().saturating_sub(5);
                    s.drain(..excess);
                }
            });
        }
        let reader = {
            let (pending, alive, stdin, name) = (pending.clone(), alive.clone(), stdin.clone(), launch.name.clone());
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        log::debug!("mcp {name}: not JSON: {}", line.chars().take(200).collect::<String>());
                        continue;
                    };
                    match (msg.get("id").cloned(), msg.get("method").and_then(Value::as_str)) {
                        // The server asks something of Waddle: only ping is answered.
                        (Some(id), Some(method)) => {
                            let reply = if method == "ping" {
                                json!({ "jsonrpc": "2.0", "id": id, "result": {} })
                            } else {
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Waddle doesn't support that" } })
                            };
                            let _ = write_line(&stdin, &reply).await;
                        }
                        (Some(id), None) => {
                            let Some(id) = id.as_u64() else { continue };
                            let reply = match msg.get("error") {
                                Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
                                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            if let Some(tx) = pending.lock().unwrap().remove(&id) {
                                let _ = tx.send(reply);
                            }
                        }
                        // Notifications (progress, list changed, logs) aren't needed.
                        _ => {}
                    }
                }
                alive.store(false, Ordering::SeqCst);
                // Anyone still waiting hears that it's gone.
                pending.lock().unwrap().clear();
            })
        };
        let client = Client { child: Mutex::new(Some(child)), stdin, pending, next_id: AtomicU64::new(1), alive, reader, stderr };
        let init = client
            .request(
                "initialize",
                json!({ "protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": { "name": "waddle", "version": env!("CARGO_PKG_VERSION") } }),
                START_TIMEOUT,
                cancel,
            )
            .await
            .map_err(|e| anyhow!("{e:#}{}", client.stderr_note()))?;
        let version = init["protocolVersion"].as_str().unwrap_or("");
        if !SPOKEN.contains(&version) {
            bail!("the server speaks MCP version \"{version}\", which Waddle doesn't know yet");
        }
        client.notify("notifications/initialized", json!({})).await?;
        Ok(client)
    }

    fn stderr_note(&self) -> String {
        let lines = self.stderr.lock().unwrap();
        match lines.last() {
            Some(last) => format!(" (it said: {last})"),
            None => String::new(),
        }
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    async fn request(&self, method: &str, params: Value, timeout: Duration, cancel: &CancellationToken) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = write_line(&self.stdin, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        let outcome = tokio::select! {
            r = rx => r.map_err(|_| anyhow!("the server stopped{}", self.stderr_note()))?.map_err(|e| anyhow!("{e}")),
            _ = tokio::time::sleep(timeout) => Err(anyhow!("no answer after {} s", timeout.as_secs())),
            _ = cancel.cancelled() => Err(anyhow!("stopped")),
        };
        if outcome.is_err() {
            self.pending.lock().unwrap().remove(&id);
            if method != "initialize" {
                // Tell the server to give up on it (harmless if it already finished; initialize can't be cancelled).
                let _ = self.notify("notifications/cancelled", json!({ "requestId": id })).await;
            }
        }
        outcome
    }

    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        write_line(&self.stdin, &json!({ "jsonrpc": "2.0", "method": method, "params": params })).await
    }

    async fn list_tools(&self, cancel: &CancellationToken) -> anyhow::Result<Vec<McpTool>> {
        let mut tools = vec![];
        let mut cursor: Option<String> = None;
        // A runaway server can't page forever.
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = self.request("tools/list", params, START_TIMEOUT, cancel).await?;
            tools.extend(page["tools"].as_array().into_iter().flatten().filter_map(McpTool::parse));
            cursor = page["nextCursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    /// Closes stdin (well-behaved servers exit), then kills what's left.
    async fn shutdown(&self) {
        self.stdin.lock().await.take();
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            if tokio::time::timeout(Duration::from_secs(2), child.wait()).await.is_err() {
                let _ = child.start_kill();
            }
        }
        self.reader.abort();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
        self.reader.abort();
    }
}

/// Turns a tools/call result into text for the model.
pub fn format_result(server: &str, result: &Value) -> (String, bool) {
    let mut parts: Vec<String> = vec![];
    for c in result["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or("").to_string()),
            Some("image") => parts.push("[an image was left out]".into()),
            Some("audio") => parts.push("[audio was left out]".into()),
            Some("resource") => match c["resource"]["text"].as_str() {
                Some(t) => parts.push(t.to_string()),
                None => parts.push(format!("[a file was left out: {}]", c["resource"]["uri"].as_str().unwrap_or("?"))),
            },
            Some("resource_link") => parts.push(format!("[link: {}]", c["uri"].as_str().unwrap_or("?"))),
            _ => {}
        }
    }
    if parts.iter().all(|p| p.trim().is_empty()) {
        if let Some(s) = result.get("structuredContent").filter(|v| !v.is_null()) {
            parts = vec![s.to_string()];
        }
    }
    let mut text = parts.join("\n");
    if text.trim().is_empty() {
        text = "(no output)".into();
    }
    if let Some((i, _)) = text.char_indices().nth(MAX_RESULT) {
        text.truncate(i);
        text.push_str("\n[cut off here]");
    }
    let failed = result["isError"].as_bool().unwrap_or(false);
    let text = if failed { format!("{server} reported an error: {text}") } else { format!("From {server}:\n{text}") };
    (text, failed)
}

/// `mcp_<server>_<tool>`, in the characters tool-calling APIs accept.
fn offered_name(server: &str, tool: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect::<String>();
    let name = format!("mcp_{}_{}", clean(server), clean(tool));
    name.chars().take(MAX_NAME).collect()
}

/// Tool schemas as the model sees them: an object, without keys some APIs reject.
fn clean_schema(schema: &Value) -> Value {
    let mut s = if schema.is_object() { schema.clone() } else { json!({ "type": "object" }) };
    if let Some(o) = s.as_object_mut() {
        o.remove("$schema");
        o.entry("type").or_insert(json!("object"));
        o.entry("properties").or_insert(json!({}));
    }
    s
}

// ---------- all servers ----------

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    /// Tool lists by server name, with the fingerprint they were listed under.
    servers: HashMap<String, (String, Vec<McpTool>)>,
}

#[derive(Default)]
struct Offered {
    /// Offered name → (server, tool name).
    names: HashMap<String, (String, String)>,
}

pub struct McpHub {
    launches: RwLock<Vec<Launch>>,
    running: tokio::sync::Mutex<HashMap<String, Arc<Client>>>,
    last_used: Mutex<HashMap<String, Instant>>,
    cache: RwLock<Cache>,
    offered: RwLock<Offered>,
    cache_path: Option<PathBuf>,
    /// The last problem starting or listing each server, for Settings.
    problems: RwLock<HashMap<String, String>>,
}

impl McpHub {
    pub fn new(cache_path: Option<PathBuf>) -> Arc<Self> {
        let cache: Cache = cache_path.as_deref().map(crate::store::load_json).unwrap_or_default();
        Arc::new(Self {
            launches: RwLock::default(),
            running: tokio::sync::Mutex::default(),
            last_used: Mutex::default(),
            cache: RwLock::new(cache),
            offered: RwLock::default(),
            cache_path,
            problems: RwLock::default(),
        })
    }

    /// The enabled servers. Servers that went away or changed are stopped.
    pub async fn configure(&self, launches: Vec<Launch>) {
        let stale: Vec<String> = {
            let old = self.launches.read().unwrap();
            old.iter().filter(|o| !launches.iter().any(|n| n == *o)).map(|o| o.name.clone()).collect()
        };
        *self.launches.write().unwrap() = launches;
        let mut running = self.running.lock().await;
        for name in stale {
            if let Some(c) = running.remove(&name) {
                c.shutdown().await;
            }
        }
        drop(running);
        self.reindex();
    }

    fn launch(&self, server: &str) -> Option<Launch> {
        self.launches.read().unwrap().iter().find(|l| l.name == server).cloned()
    }

    /// Servers that have never listed their tools (or changed since), to list in the background.
    pub fn unlisted(&self) -> Vec<String> {
        let cache = self.cache.read().unwrap();
        self.launches.read().unwrap().iter().filter(|l| cache.servers.get(&l.name).is_none_or(|(fp, _)| *fp != l.fingerprint())).map(|l| l.name.clone()).collect()
    }

    fn reindex(&self) {
        let cache = self.cache.read().unwrap();
        let mut names = HashMap::new();
        for l in self.launches.read().unwrap().iter() {
            let Some((fp, tools)) = cache.servers.get(&l.name) else { continue };
            if *fp != l.fingerprint() {
                continue;
            }
            for t in tools {
                let mut name = offered_name(&l.name, &t.name);
                let mut n = 2;
                while names.contains_key(&name) {
                    let suffix = format!("_{n}");
                    name = format!("{}{suffix}", offered_name(&l.name, &t.name).chars().take(MAX_NAME - suffix.len()).collect::<String>());
                    n += 1;
                }
                names.insert(name, (l.name.clone(), t.name.clone()));
            }
        }
        self.offered.write().unwrap().names = names;
    }

    fn save_cache(&self) {
        let Some(path) = &self.cache_path else { return };
        let json = serde_json::to_string(&*self.cache.read().unwrap());
        if let Ok(json) = json {
            if let Err(e) = crate::store::write_atomic(path, json) {
                log::warn!("couldn't save the MCP tool list: {e}");
            }
        }
    }

    /// The tools to offer the planner, from the cached lists.
    pub fn specs(&self) -> Vec<ToolSpec> {
        let offered = self.offered.read().unwrap();
        let mut specs: Vec<ToolSpec> = offered
            .names
            .iter()
            .filter_map(|(name, (server, tool))| {
                let info = self.info_for(server, tool)?;
                let description = if info.tool.description.is_empty() { format!("{tool} (from the {server} MCP server)") } else { format!("{} (from the {server} MCP server)", info.tool.description) };
                Some(ToolSpec { name: name.clone(), description, parameters: clean_schema(&info.tool.input_schema) })
            })
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    fn info_for(&self, server: &str, tool: &str) -> Option<ToolInfo> {
        let launch = self.launch(server)?;
        let cache = self.cache.read().unwrap();
        let t = cache.servers.get(server)?.1.iter().find(|t| t.name == tool)?.clone();
        Some(ToolInfo { server: server.to_string(), tool: t, trusted: launch.trusted })
    }

    /// What's behind an offered name, if it's an MCP tool.
    pub fn lookup(&self, offered: &str) -> Option<ToolInfo> {
        let (server, tool) = self.offered.read().unwrap().names.get(offered).cloned()?;
        self.info_for(&server, &tool)
    }

    /// The running client for `server`, starting it if needed (and again if it crashed).
    async fn client(&self, server: &str, cancel: &CancellationToken) -> anyhow::Result<Arc<Client>> {
        let launch = self.launch(server).ok_or_else(|| anyhow!("there's no MCP server called {server}"))?;
        let mut running = self.running.lock().await;
        if let Some(c) = running.get(server) {
            if c.is_alive() {
                return Ok(c.clone());
            }
            log::info!("mcp {server}: stopped; starting it again");
            running.remove(server);
        }
        let client = Arc::new(Client::start(&launch, cancel).await?);
        running.insert(server.to_string(), client.clone());
        Ok(client)
    }

    /// Starts `server` and lists its tools, updating the cache.
    pub async fn refresh(&self, server: &str, cancel: &CancellationToken) -> anyhow::Result<Vec<McpTool>> {
        let launch = self.launch(server).ok_or_else(|| anyhow!("there's no MCP server called {server}"))?;
        let result = async {
            let client = self.client(server, cancel).await?;
            client.list_tools(cancel).await
        }
        .await;
        self.last_used.lock().unwrap().insert(server.to_string(), Instant::now());
        match result {
            Ok(tools) => {
                self.problems.write().unwrap().remove(server);
                self.cache.write().unwrap().servers.insert(server.to_string(), (launch.fingerprint(), tools.clone()));
                self.save_cache();
                self.reindex();
                log::info!("mcp {server}: {} tools", tools.len());
                Ok(tools)
            }
            Err(e) => {
                self.problems.write().unwrap().insert(server.to_string(), format!("{e:#}"));
                Err(e)
            }
        }
    }

    /// Lists the tools of every server that hasn't yet (run in the background at start).
    pub async fn refresh_unlisted(&self) {
        for server in self.unlisted() {
            if let Err(e) = self.refresh(&server, &CancellationToken::new()).await {
                log::warn!("mcp {server}: {e:#}");
            }
        }
    }

    pub fn problem(&self, server: &str) -> Option<String> {
        self.problems.read().unwrap().get(server).cloned()
    }

    /// The cached tools of a server, for Settings.
    pub fn tools_of(&self, server: &str) -> Vec<McpTool> {
        self.cache.read().unwrap().servers.get(server).map(|(_, t)| t.clone()).unwrap_or_default()
    }

    /// Runs an offered tool. A server that crashed earlier is started again first;
    /// one that stops during the call isn't retried, since the call may have done something.
    pub async fn call(&self, offered: &str, arguments: &Value, cancel: &CancellationToken) -> anyhow::Result<ToolOutcome> {
        let info = self.lookup(offered).ok_or_else(|| anyhow!("`{offered}` isn't available any more"))?;
        let client = self.client(&info.server, cancel).await?;
        self.last_used.lock().unwrap().insert(info.server.clone(), Instant::now());
        let args = if arguments.is_object() { arguments.clone() } else { json!({}) };
        let result = client.request("tools/call", json!({ "name": info.tool.name, "arguments": args }), CALL_TIMEOUT, cancel).await;
        self.last_used.lock().unwrap().insert(info.server.clone(), Instant::now());
        let result = result.with_context(|| format!("{} ({})", info.tool.name, info.server))?;
        let (text, _failed) = format_result(&info.server, &result);
        Ok(ToolOutcome::untrusted("mcp", text))
    }

    /// Stops servers unused for `max_idle` (the app uses `IDLE_STOP`).
    pub async fn stop_idle(&self, max_idle: Duration) {
        let idle: Vec<String> = {
            let used = self.last_used.lock().unwrap();
            used.iter().filter(|(_, t)| t.elapsed() >= max_idle).map(|(s, _)| s.clone()).collect()
        };
        let mut running = self.running.lock().await;
        for server in idle {
            if let Some(c) = running.remove(&server) {
                log::info!("mcp {server}: idle; stopping it");
                c.shutdown().await;
            }
            self.last_used.lock().unwrap().remove(&server);
        }
    }

    pub async fn stop_all(&self) {
        let mut running = self.running.lock().await;
        for (_, c) in running.drain() {
            c.shutdown().await;
        }
    }

    /// How many servers are running now (for tests and the self-test).
    pub async fn running_count(&self) -> usize {
        self.running.lock().await.values().filter(|c| c.is_alive()).count()
    }
}

/// Starts a server once and lists its tools, without keeping anything (Settings → Test).
pub async fn probe(launch: &Launch) -> anyhow::Result<Vec<McpTool>> {
    let cancel = CancellationToken::new();
    let client = Client::start(launch, &cancel).await?;
    let tools = client.list_tools(&cancel).await;
    client.shutdown().await;
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_fit_tool_apis() {
        assert_eq!(offered_name("Git Hub", "create.issue"), "mcp_git_hub_create_issue");
        assert_eq!(offered_name("fs", &"x".repeat(100)).len(), MAX_NAME);
        assert!(offered_name("ü", "a/b").chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn results_become_text() {
        let r = json!({ "content": [
            { "type": "text", "text": "line one" },
            { "type": "image", "data": "...", "mimeType": "image/png" },
            { "type": "resource", "resource": { "uri": "file:///a.txt", "text": "file text" } },
            { "type": "resource_link", "uri": "file:///b.txt", "name": "b" }
        ] });
        let (text, failed) = format_result("files", &r);
        assert_eq!(text, "From files:\nline one\n[an image was left out]\nfile text\n[link: file:///b.txt]");
        assert!(!failed);
        let (text, failed) = format_result("files", &json!({ "content": [{ "type": "text", "text": "no such file" }], "isError": true }));
        assert_eq!((text.as_str(), failed), ("files reported an error: no such file", true));
        let (text, _) = format_result("w", &json!({ "content": [], "structuredContent": { "temp": 21 } }));
        assert_eq!(text, "From w:\n{\"temp\":21}");
        let (text, _) = format_result("w", &json!({ "content": [{ "type": "text", "text": "é".repeat(MAX_RESULT + 5) }] }));
        assert!(text.ends_with("[cut off here]"));
    }

    #[test]
    fn hints_are_read_with_safe_defaults() {
        let t = McpTool::parse(&json!({ "name": "read", "annotations": { "readOnlyHint": true } })).unwrap();
        assert!(t.read_only && !t.destructive);
        let t = McpTool::parse(&json!({ "name": "write" })).unwrap();
        assert!(!t.read_only && t.destructive, "no hints: may be destructive");
        let t = McpTool::parse(&json!({ "name": "add", "annotations": { "destructiveHint": false } })).unwrap();
        assert!(!t.read_only && !t.destructive);
        assert!(McpTool::parse(&json!({ "description": "nameless" })).is_none());
    }

    #[test]
    fn schemas_are_objects() {
        assert_eq!(clean_schema(&Value::Null), json!({ "type": "object", "properties": {} }));
        let s = clean_schema(&json!({ "$schema": "x", "type": "object", "properties": { "a": { "type": "string" } }, "required": ["a"] }));
        assert_eq!(s, json!({ "type": "object", "properties": { "a": { "type": "string" } }, "required": ["a"] }));
    }
}
