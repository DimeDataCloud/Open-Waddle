//! Crash-safe file saves. New contents go to a temporary file beside the old
//! one and then replace it in a single rename, so a crash or power cut leaves
//! either the old file or the new one, never half of one.

use std::io::Write;
use std::path::{Path, PathBuf};

fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp"))
}

fn write_with(path: &Path, contents: &[u8], private: bool) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(path);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut f = options.open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Replaces `path` with `contents` in one step, creating its folder if needed.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_with(path, contents.as_ref(), false)
}

/// Like `write_atomic`, readable only by the user on Unix (for the key file fallback).
pub fn write_atomic_private(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_with(path, contents.as_ref(), true)
}

/// Reads a JSON store. A file that exists but can't be parsed is moved aside to
/// `<name>.bad` (so the next save doesn't destroy it) and reported in the error.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("couldn't read {}: {e}", path.display())),
    };
    match serde_json::from_str(&text) {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let bad = path.with_file_name(format!("{name}.bad"));
            let _ = std::fs::rename(path, &bad);
            Err(format!("{} couldn't be read ({e}); it was kept as {}", path.display(), bad.display()))
        }
    }
}

/// Loads a JSON store, falling back to the default (and logging) when it's missing or unreadable.
pub fn load_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    match read_json(path) {
        Ok(v) => v.unwrap_or_default(),
        Err(e) => {
            log::warn!("{e}");
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_in_one_step_and_leaves_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub").join("facts.json");
        write_atomic(&p, "[1]").unwrap();
        write_atomic(&p, "[1,2]").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[1,2]");
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("facts.json")]);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("keys.json");
        write_atomic_private(&p, "{}").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn unreadable_json_is_kept_aside() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.json");
        assert_eq!(read_json::<Vec<u8>>(&p), Ok(None));
        std::fs::write(&p, "{not json").unwrap();
        let err = read_json::<Vec<u8>>(&p).unwrap_err();
        assert!(err.contains("settings.json.bad"), "{err}");
        assert!(!p.exists());
        assert_eq!(std::fs::read_to_string(d.path().join("settings.json.bad")).unwrap(), "{not json");
        assert_eq!(load_json::<Vec<u8>>(&p), Vec::<u8>::new());
    }
}
