//! File access within a scope. Relative paths mean the workspace folder.
//! Absolute and `~/` paths may reach the folders the user allowed: reading in
//! the read folders (user folders, Drive for desktop), writing only in the
//! workspace and the write folders. Secrets (app data, SSH keys, browser
//! profiles, password databases, `.env` files) are never reachable. Escapes
//! (`..`, symlinks pointing outside) are rejected before touching the disk.

use anyhow::{bail, Context};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::safety::SafetyContext;

const MAX_READ: usize = 64 * 1024;
const MAX_WRITE: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

pub struct Workspace {
    root: PathBuf,
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
}

/// Folders and files that hold secrets: never read or written, whatever the scope.
pub fn is_denied(path: &Path) -> bool {
    const DIRS: &[&str] = &[
        "appdata", ".ssh", ".gnupg", ".aws", ".azure", ".kube", ".docker", ".password-store", "keychains",
        "dev.waddle.app", "user data", "profiles", ".mozilla", "google-chrome", "chromium", ".git",
    ];
    const FILES: &[&str] = &[".env", ".netrc", ".git-credentials", ".npmrc", ".pypirc", "credentials", "known_hosts", "authorized_keys"];
    const EXTS: &[&str] = &["kdbx", "kdb", "pem", "key", "p12", "pfx", "ppk", "keychain", "gpg", "asc", "ovpn"];
    let comps: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    let Some(name) = comps.last() else { return false };
    if comps.iter().any(|d| DIRS.contains(&d.as_str())) {
        return true;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    FILES.contains(&name.as_str()) || name.starts_with(".env.") || name.starts_with("id_rsa") || name.starts_with("id_ed25519") || name.starts_with("id_ecdsa") || EXTS.contains(&ext)
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Lexically normalises a path, then resolves symlinks on its deepest existing ancestor.
fn real_path(path: &Path, shown: &str) -> anyhow::Result<PathBuf> {
    let mut normal = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if !normal.pop() {
                    bail!("`{shown}` goes above the top of the disk");
                }
            }
            Component::CurDir => {}
            other => normal.push(other.as_os_str()),
        }
    }
    let mut existing = normal.as_path();
    let mut tail = vec![];
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                existing = parent;
            }
            _ => bail!("`{shown}` doesn't exist"),
        }
    }
    let mut real = existing.canonicalize()?;
    for name in tail.iter().rev() {
        real.push(name);
    }
    Ok(real)
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        std::fs::create_dir_all(root).with_context(|| format!("creating workspace {}", root.display()))?;
        Ok(Self { root: root.canonicalize()?, read_roots: vec![], write_roots: vec![] })
    }

    /// Adds the folders Waddle may read in and write in, besides the workspace. Missing folders are skipped.
    pub fn with_roots(mut self, read: &[PathBuf], write: &[PathBuf]) -> Self {
        let real = |v: &[PathBuf]| v.iter().filter_map(|p| p.canonicalize().ok()).filter(|p| p.is_dir()).collect::<Vec<_>>();
        self.read_roots = real(read);
        self.write_roots = real(write);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    pub fn write_roots(&self) -> &[PathBuf] {
        &self.write_roots
    }

    /// Whether this scope reaches beyond the workspace.
    pub fn is_wide(&self) -> bool {
        !self.read_roots.is_empty() || !self.write_roots.is_empty()
    }

    /// Resolves a path for reading or writing. Relative paths are in the workspace;
    /// absolute and `~/` paths must be inside an allowed folder.
    pub fn resolve_for(&self, path: &str, access: Access) -> anyhow::Result<PathBuf> {
        let path = path.trim();
        let expanded = match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
            Some(rest) => home().ok_or_else(|| anyhow::anyhow!("no home folder"))?.join(rest),
            None if path == "~" => home().ok_or_else(|| anyhow::anyhow!("no home folder"))?,
            None => PathBuf::from(path),
        };
        if !(expanded.is_absolute() || expanded.has_root()) {
            return self.resolve(path);
        }
        let real = real_path(&expanded, path)?;
        if is_denied(&real) {
            bail!("`{path}` is private (app data, keys, passwords or browser profiles), so Waddle doesn't touch it");
        }
        let inside = |roots: &[PathBuf]| roots.iter().any(|r| real.starts_with(r));
        let ok = real.starts_with(&self.root)
            || inside(&self.write_roots)
            || (access == Access::Read && inside(&self.read_roots));
        if !ok {
            let allowed: Vec<String> = std::iter::once(&self.root)
                .chain(&self.write_roots)
                .chain(if access == Access::Read { self.read_roots.iter() } else { [].iter() })
                .map(|p| p.display().to_string())
                .collect();
            bail!(
                "`{path}` is outside the folders Waddle may {} ({}). The user can add folders in Settings.",
                if access == Access::Read { "read" } else { "change" },
                allowed.join(", ")
            );
        }
        Ok(real)
    }

    /// Resolves a workspace-relative path, refusing anything outside the root.
    pub fn resolve(&self, rel: &str) -> anyhow::Result<PathBuf> {
        let rel = rel.trim();
        let candidate = Path::new(rel);
        let joined = if candidate.is_absolute() || candidate.has_root() { candidate.to_path_buf() } else { self.root.join(candidate) };
        // Lexical normalisation first, so `a/../../x` cannot slip through.
        let mut normal = PathBuf::new();
        for comp in joined.components() {
            match comp {
                Component::ParentDir => {
                    if !normal.pop() {
                        bail!("`{rel}` is outside the workspace");
                    }
                }
                Component::CurDir => {}
                other => normal.push(other.as_os_str()),
            }
        }
        // Then resolve symlinks on the deepest existing ancestor.
        let mut existing = normal.as_path();
        let mut tail = vec![];
        while !existing.exists() {
            match (existing.parent(), existing.file_name()) {
                (Some(parent), Some(name)) => {
                    tail.push(name.to_os_string());
                    existing = parent;
                }
                _ => bail!("`{rel}` is outside the workspace"),
            }
        }
        let mut real = existing.canonicalize()?;
        for name in tail.iter().rev() {
            real.push(name);
        }
        if !real.starts_with(&self.root) {
            bail!("`{rel}` is outside the workspace ({})", self.root.display());
        }
        Ok(real)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.resolve_for(rel, Access::Read).map(|p| p.exists()).unwrap_or(false)
    }

    pub fn read_file(&self, rel: &str) -> anyhow::Result<String> {
        let path = self.resolve_for(rel, Access::Read)?;
        let bytes = std::fs::read(&path).with_context(|| format!("reading {rel}"))?;
        let truncated = bytes.len() > MAX_READ;
        let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_READ)]).into_owned();
        if truncated {
            text.push_str(&format!("\n[truncated: showing the first {MAX_READ} of {} bytes]", bytes.len()));
        }
        Ok(text)
    }

    pub fn write_file(&self, rel: &str, content: &str) -> anyhow::Result<String> {
        if content.len() > MAX_WRITE {
            bail!("content is larger than {MAX_WRITE} bytes");
        }
        let path = self.writable_file(rel)?;
        let existed = self.bin_old_copy(&path)?;
        std::fs::write(&path, content).with_context(|| format!("writing {rel}"))?;
        Ok(format!(
            "{} {rel} ({} bytes). The file is saved; there's nothing to check.",
            if existed { "Overwrote (the old copy is in the Recycle Bin)" } else { "Created" },
            content.len()
        ))
    }

    /// A file path Waddle may write, with its folder created.
    pub fn writable_file(&self, rel: &str) -> anyhow::Result<PathBuf> {
        let path = self.resolve_for(rel, Access::Write)?;
        if path == self.root || path.is_dir() {
            bail!("a file path is required");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(path)
    }

    /// Before overwriting, the old copy goes to the Recycle Bin. Returns whether there was one.
    pub fn bin_old_copy(&self, path: &Path) -> anyhow::Result<bool> {
        if !path.exists() {
            return Ok(false);
        }
        trash::delete(path).with_context(|| format!("moving the old {} to the Recycle Bin", path.display()))?;
        Ok(true)
    }

    /// Moves or renames within the folders Waddle may change. Never overwrites.
    pub fn move_file(&self, from: &str, to: &str) -> anyhow::Result<String> {
        let src = self.resolve_for(from, Access::Write)?;
        anyhow::ensure!(src.exists(), "`{from}` doesn't exist");
        anyhow::ensure!(src != self.root, "the workspace itself can't be moved");
        let mut dest = self.resolve_for(to, Access::Write)?;
        if dest.is_dir() {
            dest = dest.join(src.file_name().unwrap_or_default());
        }
        anyhow::ensure!(!dest.exists(), "`{}` already exists; pick another name", dest.display());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&src, &dest).or_else(|_| std::fs::copy(&src, &dest).and_then(|_| std::fs::remove_file(&src)).map(|_| ()))
            .with_context(|| format!("moving {} to {}", src.display(), dest.display()))?;
        Ok(format!("Moved {} to {}.", src.display(), dest.display()))
    }

    /// Renames a file in place (same folder).
    pub fn rename_file(&self, path: &str, new_name: &str) -> anyhow::Result<String> {
        let new_name = new_name.trim();
        anyhow::ensure!(!new_name.is_empty() && !new_name.contains(['/', '\\']) && new_name != ".." , "`new_name` must be a plain file name");
        let src = self.resolve_for(path, Access::Write)?;
        let dest = src.with_file_name(new_name);
        self.move_file(&src.display().to_string(), &dest.display().to_string())
    }

    /// Sends a file or folder to the Recycle Bin.
    pub fn delete_file(&self, path: &str) -> anyhow::Result<String> {
        let p = self.resolve_for(path, Access::Write)?;
        anyhow::ensure!(p.exists(), "`{path}` doesn't exist");
        anyhow::ensure!(p != self.root && !self.write_roots.contains(&p), "a whole allowed folder can't be deleted");
        trash::delete(&p).with_context(|| format!("moving {} to the Recycle Bin", p.display()))?;
        Ok(format!("Moved {} to the Recycle Bin (it can be restored from there).", p.display()))
    }

    /// Finds files by name, then by content (small text files), in the given folder
    /// or every readable one. Bounded in time and number of entries.
    pub fn find_files(&self, query: &str, root: Option<&str>, ext: Option<&str>, within_days: Option<u64>) -> anyhow::Result<Vec<Found>> {
        const MAX_VISITS: usize = 30_000;
        const MAX_DEPTH: usize = 8;
        const MAX_RESULTS: usize = 30;
        const BUDGET: Duration = Duration::from_secs(4);
        const SMALL: &[&str] = &["my", "the", "a", "an", "of", "for", "from", "in", "on", "and", "to", "file", "files", "document"];
        let words: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .filter(|w| !w.is_empty() && !SMALL.contains(&w.as_str()))
            .collect();
        // Half the words (at least one) must show up in a name for it to count.
        let needed = (words.len() / 2).max(1);
        let ext = ext.map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty());
        let newer = within_days.filter(|d| *d > 0).map(|d| SystemTime::now() - Duration::from_secs(d * 86_400));
        let roots: Vec<PathBuf> = match root.map(str::trim).filter(|r| !r.is_empty()) {
            Some(r) => vec![self.resolve_for(r, Access::Read)?],
            None => std::iter::once(self.root.clone()).chain(self.write_roots.iter().cloned()).chain(self.read_roots.iter().cloned()).collect(),
        };
        let started = Instant::now();
        let mut by_name = vec![];
        let mut candidates = vec![];
        let mut visits = 0;
        let mut queue: std::collections::VecDeque<(PathBuf, usize)> = roots.into_iter().map(|r| (r, 0)).collect();
        let mut seen = std::collections::HashSet::new();
        while let Some((dir, depth)) = queue.pop_front() {
            if visits >= MAX_VISITS || started.elapsed() > BUDGET || !seen.insert(dir.clone()) {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                visits += 1;
                let path = e.path();
                let name = e.file_name().to_string_lossy().to_lowercase();
                if name.starts_with('.') || name == "node_modules" || is_denied(&path) {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                if meta.is_dir() {
                    if depth < MAX_DEPTH {
                        queue.push_back((path, depth + 1));
                    }
                    continue;
                }
                let file_ext = name.rsplit_once('.').map(|(_, x)| x.to_string()).unwrap_or_default();
                if ext.as_ref().is_some_and(|x| *x != file_ext) {
                    continue;
                }
                let modified = meta.modified().ok();
                if newer.is_some_and(|n| modified.is_none_or(|m| m < n)) {
                    continue;
                }
                let found = Found { path, size: meta.len(), modified, matched_content: false };
                let hits = name_hits(&name, &words);
                if words.is_empty() || hits >= needed {
                    by_name.push((hits, found));
                } else if meta.len() < 1_000_000 && TEXT_EXTS.contains(&file_ext.as_str()) {
                    candidates.push(found);
                }
            }
        }
        // Best name matches first, then the newest.
        by_name.sort_by_key(|(hits, f)| (std::cmp::Reverse(*hits), std::cmp::Reverse(f.modified)));
        let mut by_name: Vec<Found> = by_name.into_iter().map(|(_, f)| f).take(MAX_RESULTS).collect();
        // Not enough by name: look inside small text files too.
        if by_name.len() < 5 && !words.is_empty() {
            candidates.sort_by_key(|f| std::cmp::Reverse(f.modified));
            for mut c in candidates.into_iter().take(2_000) {
                if by_name.len() >= MAX_RESULTS || started.elapsed() > BUDGET * 2 {
                    break;
                }
                if let Ok(text) = std::fs::read_to_string(&c.path) {
                    let text = text.to_lowercase();
                    if words.iter().all(|w| text.contains(w)) {
                        c.matched_content = true;
                        by_name.push(c);
                    }
                }
            }
        }
        Ok(by_name)
    }

    pub fn list_dir(&self, rel: &str) -> anyhow::Result<String> {
        let path = self.resolve_for(if rel.trim().is_empty() { "." } else { rel }, Access::Read)?;
        let mut names: Vec<String> = std::fs::read_dir(&path)
            .with_context(|| format!("listing {rel}"))?
            .filter_map(Result::ok)
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { format!("{name}/") } else { name }
            })
            .collect();
        names.sort();
        let total = names.len();
        names.truncate(MAX_ENTRIES);
        let mut out = if names.is_empty() { "(empty folder)".to_string() } else { names.join("\n") };
        if total > MAX_ENTRIES {
            out.push_str(&format!("\n[{} more entries not shown]", total - MAX_ENTRIES));
        }
        Ok(out)
    }
}

/// How many query words a file name contains. A word also matches a name part it starts
/// with or that starts with it (3+ letters), so "october" finds "oct".
fn name_hits(name: &str, words: &[String]) -> usize {
    let parts: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).collect();
    words
        .iter()
        .filter(|w| name.contains(w.as_str()) || parts.iter().any(|p| (p.len() >= 3 && w.starts_with(p)) || (w.len() >= 3 && p.starts_with(w.as_str()))))
        .count()
}

/// Plain-text files worth searching inside.
const TEXT_EXTS: &[&str] = &["txt", "md", "csv", "json", "html", "htm", "xml", "log", "ini", "yaml", "yml", "toml", "rtf", "tex"];

#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub matched_content: bool,
}

impl SafetyContext for Workspace {
    fn file_exists(&self, path: &str) -> bool {
        self.exists(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().join("ws")).unwrap();
        (dir, ws)
    }

    #[test]
    fn write_read_list_roundtrip() {
        let (_d, ws) = ws();
        assert!(ws.write_file("notes/hello.txt", "hi").unwrap().starts_with("Created"));
        assert!(ws.exists("notes/hello.txt"));
        assert_eq!(ws.read_file("notes/hello.txt").unwrap(), "hi");
        assert_eq!(ws.list_dir("").unwrap(), "notes/");
        assert!(ws.write_file("notes/hello.txt", "again").unwrap().starts_with("Overwrote"));
    }

    #[test]
    fn escapes_are_rejected() {
        let (d, ws) = ws();
        std::fs::write(d.path().join("secret.txt"), "s").unwrap();
        for bad in ["../secret.txt", "a/../../secret.txt", d.path().join("secret.txt").to_str().unwrap()] {
            assert!(ws.read_file(bad).is_err(), "{bad}");
            assert!(ws.write_file(bad, "x").is_err(), "{bad}");
        }
        assert!(!ws.exists("../secret.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_out_of_workspace_is_rejected() {
        let (d, ws) = ws();
        std::fs::create_dir(d.path().join("outside")).unwrap();
        std::os::unix::fs::symlink(d.path().join("outside"), ws.root().join("link")).unwrap();
        assert!(ws.write_file("link/x.txt", "x").is_err());
    }

    #[test]
    fn scope_reads_wide_writes_narrow_and_never_touches_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("Documents");
        let drop = dir.path().join("Drop");
        let other = dir.path().join("Other");
        for d in [&docs, &drop, &other, &docs.join(".ssh"), &docs.join("AppData")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(docs.join("plan.txt"), "the plan").unwrap();
        std::fs::write(docs.join(".ssh").join("id_ed25519"), "KEY").unwrap();
        std::fs::write(docs.join("vault.kdbx"), "x").unwrap();
        std::fs::write(docs.join(".env"), "TOKEN=1").unwrap();
        std::fs::write(other.join("x.txt"), "x").unwrap();
        let ws = Workspace::new(dir.path().join("ws")).unwrap().with_roots(std::slice::from_ref(&docs), std::slice::from_ref(&drop));
        let p = |x: &Path| x.display().to_string();

        assert_eq!(ws.read_file(&p(&docs.join("plan.txt"))).unwrap(), "the plan", "read folders are readable");
        assert!(ws.write_file(&p(&docs.join("new.txt")), "x").unwrap_err().to_string().contains("change"), "but not writable");
        assert!(ws.write_file(&p(&drop.join("new.txt")), "x").is_ok(), "write folders are");
        assert!(ws.read_file(&p(&other.join("x.txt"))).is_err(), "other folders aren't reachable");
        for secret in [docs.join(".ssh").join("id_ed25519"), docs.join("vault.kdbx"), docs.join(".env"), docs.join("AppData")] {
            let err = ws.read_file(&p(&secret)).unwrap_err().to_string();
            assert!(err.contains("private"), "{err}");
        }
        assert!(ws.read_file(&format!("{}/../Other/x.txt", p(&docs))).is_err(), "no climbing out");
        assert!(ws.write_file("rel.txt", "workspace").is_ok(), "relative paths still mean the workspace");
    }

    #[test]
    fn overwrites_move_files_and_deletes_go_through_the_recycle_bin() {
        let (_d, ws) = ws();
        ws.write_file("a.txt", "one").unwrap();
        assert!(ws.write_file("a.txt", "two").unwrap().contains("Recycle Bin"));
        assert_eq!(ws.read_file("a.txt").unwrap(), "two");
        ws.write_file("b.txt", "b").unwrap();
        assert!(ws.move_file("b.txt", "a.txt").is_err(), "moves never overwrite");
        assert!(ws.move_file("b.txt", "sub/c.txt").is_ok());
        assert!(ws.exists("sub/c.txt") && !ws.exists("b.txt"));
        assert!(ws.rename_file("sub/c.txt", "d.txt").is_ok());
        assert!(ws.rename_file("sub/d.txt", "../escape.txt").is_err());
        assert!(ws.delete_file("sub/d.txt").unwrap().contains("Recycle Bin"));
        assert!(!ws.exists("sub/d.txt"));
        assert!(ws.delete_file(".").is_err(), "not the workspace itself");
    }

    #[test]
    fn finds_by_name_first_then_by_content() {
        let (_d, ws) = ws();
        ws.write_file("2026/Invoice March.pdf", "%PDF").unwrap();
        ws.write_file("notes/meeting.md", "Discussed the invoice for March with Ana").unwrap();
        ws.write_file("notes/other.md", "nothing here").unwrap();
        let found = ws.find_files("invoice march", None, None, None).unwrap();
        let names: Vec<String> = found.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Invoice March.pdf", "meeting.md"]);
        assert!(!found[0].matched_content && found[1].matched_content);
        assert_eq!(ws.find_files("invoice", None, Some("md"), None).unwrap().len(), 1, "extension filter");
        ws.write_file("Papers/train-ticket-oct.pdf", "%PDF").unwrap();
        let found = ws.find_files("my train ticket receipt from October", None, None, Some(0)).unwrap();
        assert!(found[0].path.ends_with("train-ticket-oct.pdf"), "partial and prefix matches count, and 0 days means any age: {found:?}");
    }

    #[test]
    fn large_files_are_truncated_on_read() {
        let (_d, ws) = ws();
        ws.write_file("big.txt", &"a".repeat(MAX_READ + 10)).unwrap();
        assert!(ws.read_file("big.txt").unwrap().contains("[truncated"));
    }
}
