//! Workspace-confined file access. Every path is resolved relative to the
//! workspace root; anything that escapes it (`..`, absolute paths, symlinks
//! pointing outside) is rejected before touching the disk.

use anyhow::{bail, Context};
use std::path::{Component, Path, PathBuf};

use crate::safety::SafetyContext;

const MAX_READ: usize = 64 * 1024;
const MAX_WRITE: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 200;

pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        std::fs::create_dir_all(root).with_context(|| format!("creating workspace {}", root.display()))?;
        Ok(Self { root: root.canonicalize()? })
    }

    pub fn root(&self) -> &Path {
        &self.root
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
        self.resolve(rel).map(|p| p.exists()).unwrap_or(false)
    }

    pub fn read_file(&self, rel: &str) -> anyhow::Result<String> {
        let path = self.resolve(rel)?;
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
        let path = self.resolve(rel)?;
        if path == self.root {
            bail!("a file path is required");
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existed = path.exists();
        std::fs::write(&path, content).with_context(|| format!("writing {rel}"))?;
        Ok(format!("{} {rel} ({} bytes). It's saved; no need to read it back.", if existed { "Overwrote" } else { "Created" }, content.len()))
    }

    pub fn list_dir(&self, rel: &str) -> anyhow::Result<String> {
        let path = self.resolve(if rel.trim().is_empty() { "." } else { rel })?;
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
    fn large_files_are_truncated_on_read() {
        let (_d, ws) = ws();
        ws.write_file("big.txt", &"a".repeat(MAX_READ + 10)).unwrap();
        assert!(ws.read_file("big.txt").unwrap().contains("[truncated"));
    }
}
