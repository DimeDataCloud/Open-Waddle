//! The link to Waddle's Chrome extension. Chrome starts `waddle.exe` with the
//! extension's origin as its argument (native messaging); that copy only relays
//! Chrome's stdin/stdout to the running app over a named pipe (a Unix socket
//! elsewhere). No network port is opened, and only the extension with Waddle's
//! fixed id may start the relay.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// The native-messaging host name the extension connects to.
pub const HOST_NAME: &str = "dev.waddle.app";
/// Fixed by the public key in extension/manifest.json, so it's the same on every computer.
pub const EXTENSION_ID: &str = "bjpppaeapoinfgpfgdgejapeckaflici";
/// Chrome's limit for a message to the extension is 1 MB; pages are far smaller.
const MAX_FRAME: usize = 8 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the app listens for the relay.
pub fn socket_name() -> String {
    if let Ok(p) = std::env::var("WADDLE_BROWSER_SOCKET") {
        return p;
    }
    #[cfg(windows)]
    {
        let user: String = std::env::var("USERNAME").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        format!(r"\\.\pipe\waddle-browser-{user}")
    }
    #[cfg(not(windows))]
    {
        let dir = std::env::var("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        let user = std::env::var("USER").unwrap_or_default();
        dir.join(format!("waddle-browser-{user}.sock")).display().to_string()
    }
}

/// Native messaging framing: a 4-byte length in native (little-endian) order, then UTF-8 JSON.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("message of {n} bytes is too big")));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, body: &[u8]) -> std::io::Result<()> {
    w.write_all(&(body.len() as u32).to_le_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await
}

/// The running app's end: requests out, answers back by id.
pub struct BrowserLink {
    out: Mutex<Option<(u64, mpsc::UnboundedSender<Vec<u8>>)>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next: AtomicU64,
    version: Mutex<Option<String>>,
}

impl BrowserLink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { out: Mutex::new(None), pending: Mutex::default(), next: AtomicU64::new(1), version: Mutex::new(None) })
    }

    pub fn connected(&self) -> bool {
        self.out.lock().unwrap().is_some()
    }

    pub fn version(&self) -> Option<String> {
        self.version.lock().unwrap().clone()
    }

    /// Sends a command to the extension and waits for its answer.
    pub async fn request(&self, cmd: &str, args: Value) -> anyhow::Result<Value> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let out = self.out.lock().unwrap();
            let Some((_, sender)) = out.as_ref() else {
                anyhow::bail!("Chrome isn't connected: open Chrome, and set up Waddle's extension in Settings if you haven't");
            };
            self.pending.lock().unwrap().insert(id, tx);
            sender.send(serde_json::to_vec(&json!({ "id": id, "cmd": cmd, "args": args }))?).map_err(|_| anyhow::anyhow!("Chrome disconnected"))?;
        }
        let answer = match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => anyhow::bail!("Chrome disconnected"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                anyhow::bail!("Chrome didn't answer in time");
            }
        };
        if answer["ok"].as_bool() == Some(true) {
            Ok(answer["result"].clone())
        } else {
            anyhow::bail!("Chrome: {}", answer["error"].as_str().unwrap_or("unknown error"))
        }
    }

    /// One connection from the relay, until it closes. The newest connection wins.
    pub async fn session<S: AsyncRead + AsyncWrite + Send + 'static>(self: Arc<Self>, stream: S) {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let epoch = self.next.fetch_add(1, Ordering::SeqCst);
        *self.out.lock().unwrap() = Some((epoch, tx));
        let writing = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
            }
        });
        loop {
            let frame = match read_frame(&mut reader).await {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    log::warn!("chrome link: {e}");
                    break;
                }
            };
            let Ok(msg) = serde_json::from_slice::<Value>(&frame) else { continue };
            if msg["type"] == "hello" {
                let v = msg["version"].as_str().unwrap_or("?").to_string();
                log::info!("Chrome extension {v} connected");
                *self.version.lock().unwrap() = Some(v);
            } else if let Some(id) = msg["id"].as_u64() {
                if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(msg);
                }
            }
        }
        writing.abort();
        let mut out = self.out.lock().unwrap();
        if out.as_ref().is_some_and(|(e, _)| *e == epoch) {
            *out = None;
            *self.version.lock().unwrap() = None;
            self.pending.lock().unwrap().clear();
            log::info!("Chrome extension disconnected");
        }
    }

    /// Listens for the relay for as long as the app runs.
    pub fn serve(self: &Arc<Self>) {
        let link = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = link.listen().await {
                log::warn!("chrome link stopped: {e:#}");
            }
        });
    }

    #[cfg(windows)]
    async fn listen(self: Arc<Self>) -> anyhow::Result<()> {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = socket_name();
        let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)?;
        loop {
            server.connect().await?;
            let conn = server;
            server = ServerOptions::new().reject_remote_clients(true).create(&name)?;
            tokio::spawn(self.clone().session(conn));
        }
    }

    #[cfg(not(windows))]
    async fn listen(self: Arc<Self>) -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let path = socket_name();
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        loop {
            let (conn, _) = listener.accept().await?;
            tokio::spawn(self.clone().session(conn));
        }
    }
}

/// The relay Chrome starts: copies whole messages between Chrome and the running app.
pub fn run_native_host() -> i32 {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return 1 };
    rt.block_on(async {
        #[cfg(windows)]
        let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(socket_name());
        #[cfg(not(windows))]
        let stream = tokio::net::UnixStream::connect(socket_name()).await;
        // Waddle isn't running: Chrome sees the port close and tries again later.
        let Ok(stream) = stream else { return 2 };
        relay(tokio::io::stdin(), tokio::io::stdout(), stream).await;
        0
    })
}

/// Chrome's side (stdin/stdout) and the app's side, frame by frame, until either closes.
pub async fn relay<I, O, S>(mut from_chrome: I, mut to_chrome: O, app: S)
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite,
{
    let (mut from_app, mut to_app) = tokio::io::split(app);
    let up = async {
        while let Ok(Some(f)) = read_frame(&mut from_chrome).await {
            if write_frame(&mut to_app, &f).await.is_err() {
                break;
            }
        }
    };
    let down = async {
        while let Ok(Some(f)) = read_frame(&mut from_app).await {
            if write_frame(&mut to_chrome, &f).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
}

/// The host manifest Chrome reads to find and trust the relay.
pub fn host_manifest(exe: &Path) -> Value {
    json!({
        "name": HOST_NAME,
        "description": "Waddle desktop app",
        "path": exe,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")]
    })
}

/// Tells Chrome (and Edge) where the relay is. Runs at every start, so a moved
/// or updated app keeps working.
pub fn register_host(data_dir: &Path) -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let manifest = serde_json::to_string_pretty(&host_manifest(&exe))?;
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(format!("{HOST_NAME}.json"));
    std::fs::write(&path, &manifest)?;
    #[cfg(windows)]
    for browser in [r"Software\Google\Chrome", r"Software\Microsoft\Edge"] {
        crate::desktop::windows::set_user_registry_default(&format!(r"{browser}\NativeMessagingHosts\{HOST_NAME}"), &path.to_string_lossy())?;
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let dirs: &[&str] = if cfg!(target_os = "macos") {
            &["Library/Application Support/Google/Chrome/NativeMessagingHosts"]
        } else {
            &[".config/google-chrome/NativeMessagingHosts", ".config/chromium/NativeMessagingHosts"]
        };
        for d in dirs {
            let dir = home.join(d);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join(format!("{HOST_NAME}.json")), &manifest)?;
        }
    }
    Ok(path)
}

/// The extension travels inside the app, so it always matches this version.
const EXTENSION_FILES: &[(&str, &[u8])] = &[
    ("manifest.json", include_bytes!("../../extension/manifest.json")),
    ("icon128.png", include_bytes!("../../extension/icon128.png")),
    ("build/background.js", include_bytes!("../../extension/build/background.js")),
    ("build/page.js", include_bytes!("../../extension/build/page.js")),
];

/// Writes the extension where Chrome can load it unpacked from. Returns that folder.
pub fn install_extension(data_dir: &Path) -> anyhow::Result<PathBuf> {
    let dest = data_dir.join("extension");
    for (rel, bytes) in EXTENSION_FILES {
        let to = dest.join(rel);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&to, bytes)?;
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_relay_carries_whole_messages_both_ways() {
        let link = BrowserLink::new();
        // The app's end of the pipe, and a pretend Chrome talking to the relay.
        let (app_end, relay_end) = tokio::io::duplex(1 << 16);
        let (chrome_out, relay_in) = tokio::io::duplex(1 << 16);
        let (relay_out, chrome_in) = tokio::io::duplex(1 << 16);
        tokio::spawn(link.clone().session(app_end));
        tokio::spawn(relay(relay_in, relay_out, relay_end));

        // The extension says hello, then answers whatever comes in.
        let extension = tokio::spawn(async move {
            let (mut from_relay, mut to_relay) = (chrome_in, chrome_out);
            write_frame(&mut to_relay, br#"{"type":"hello","version":"0.1.12"}"#).await.unwrap();
            while let Ok(Some(f)) = read_frame(&mut from_relay).await {
                let req: Value = serde_json::from_slice(&f).unwrap();
                let answer = match req["cmd"].as_str().unwrap() {
                    "read" => json!({ "id": req["id"], "ok": true, "result": { "title": "Ducks", "url": "https://ducks.example", "text": "x".repeat(20_000) } }),
                    _ => json!({ "id": req["id"], "ok": false, "error": "no such element" }),
                };
                write_frame(&mut to_relay, &serde_json::to_vec(&answer).unwrap()).await.unwrap();
            }
        });

        for _ in 0..50 {
            if link.connected() && link.version().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(link.version().as_deref(), Some("0.1.12"));
        let page = link.request("read", json!({ "mode": "text" })).await.unwrap();
        assert_eq!(page["text"].as_str().unwrap().len(), 20_000, "long messages arrive whole");
        let err = link.request("click", json!({ "element": "e9" })).await.unwrap_err();
        assert!(err.to_string().contains("no such element"), "{err}");
        extension.abort();
    }

    #[tokio::test]
    async fn without_chrome_requests_fail_fast() {
        let link = BrowserLink::new();
        let err = link.request("read", json!({})).await.unwrap_err();
        assert!(err.to_string().contains("isn't connected"));
    }

    #[test]
    fn the_manifest_trusts_only_waddles_extension() {
        let m = host_manifest(Path::new("/opt/waddle/waddle"));
        assert_eq!(m["allowed_origins"], json!(["chrome-extension://bjpppaeapoinfgpfgdgejapeckaflici/"]));
        assert_eq!(m["type"], "stdio");
        let manifest: Value = serde_json::from_str(include_str!("../../extension/manifest.json")).unwrap();
        assert!(manifest["key"].as_str().is_some_and(|k| k.len() > 300), "the fixed key keeps the extension id stable");
    }
}
