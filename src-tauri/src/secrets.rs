//! API keys live in the OS keychain (Windows Credential Manager, macOS
//! Keychain, Secret Service on Linux). Environment variables work as an
//! override for development. If no keychain is available (headless Linux),
//! keys fall back to a file readable only by the current user.

use std::path::{Path, PathBuf};

const SERVICE: &str = "dev.waddle.app";

#[derive(Debug, Clone, Copy)]
pub enum Secret {
    LlmKey,
    SttKey,
    /// Keeps Waddle signed in to Google.
    GoogleRefreshToken,
    /// The Desktop OAuth client's secret from the user's Google Cloud project.
    GoogleClient,
}

impl Secret {
    fn account(self) -> &'static str {
        match self {
            Secret::LlmKey => "llm_api_key",
            Secret::SttKey => "stt_api_key",
            Secret::GoogleRefreshToken => "google_refresh_token",
            Secret::GoogleClient => "google_client_secret",
        }
    }
    fn env_vars(self) -> &'static [&'static str] {
        match self {
            Secret::LlmKey => &["WADDLE_API_KEY", "OPENROUTER_API_KEY"],
            Secret::SttKey => &["WADDLE_STT_API_KEY", "GROQ_API_KEY"],
            Secret::GoogleRefreshToken | Secret::GoogleClient => &[],
        }
    }
}

/// Keys are plain ASCII; drops spaces, line breaks and the invisible byte-order
/// mark some editors put at the start of a saved file.
fn clean(v: &str) -> String {
    v.chars().filter(|c| c.is_ascii_graphic()).collect()
}

pub struct Secrets {
    fallback_file: PathBuf,
}

impl Secrets {
    pub fn new(config_dir: &Path) -> Self {
        Self { fallback_file: config_dir.join("secrets.json") }
    }

    pub fn get(&self, s: Secret) -> Option<String> {
        for var in s.env_vars() {
            if let Ok(v) = std::env::var(var) {
                let v = clean(&v);
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        if let Ok(entry) = keyring::Entry::new(SERVICE, s.account()) {
            if let Ok(v) = entry.get_password() {
                return Some(clean(&v));
            }
        }
        self.read_file().get(s.account()).and_then(|v| v.as_str()).map(clean)
    }

    /// Stores (or with an empty value, removes) a key. Returns where it went.
    pub fn set(&self, s: Secret, value: &str) -> anyhow::Result<&'static str> {
        let value = clean(value);
        let value = value.as_str();
        let keychain = keyring::Entry::new(SERVICE, s.account());
        if value.is_empty() {
            if let Ok(e) = keychain {
                let _ = e.delete_credential();
            }
            self.write_file(s, None)?;
            return Ok("removed");
        }
        match keychain.and_then(|e| e.set_password(value)) {
            Ok(()) => {
                self.write_file(s, None)?;
                Ok("keychain")
            }
            Err(e) => {
                log::warn!("keychain unavailable ({e}); storing key in a user-only file");
                self.write_file(s, Some(value))?;
                Ok("file")
            }
        }
    }

    /// Self-test: whether the OS keychain can store, read and delete an entry.
    pub fn probe_keychain() -> anyhow::Result<()> {
        let entry = keyring::Entry::new(SERVICE, "selftest")?;
        entry.set_password("ok")?;
        let back = entry.get_password()?;
        let _ = entry.delete_credential();
        anyhow::ensure!(back == "ok", "keychain returned a different value");
        Ok(())
    }

    fn read_file(&self) -> serde_json::Map<String, serde_json::Value> {
        std::fs::read_to_string(&self.fallback_file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn write_file(&self, s: Secret, value: Option<&str>) -> anyhow::Result<()> {
        let mut map = self.read_file();
        match value {
            Some(v) => {
                map.insert(s.account().into(), v.into());
            }
            None => {
                if map.remove(s.account()).is_none() {
                    return Ok(());
                }
            }
        }
        waddle_core::store::write_atomic_private(&self.fallback_file, serde_json::to_string_pretty(&map)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pasted_keys_lose_invisible_characters() {
        assert_eq!(super::clean("\u{feff}sk-or-v1-abc123\r\n "), "sk-or-v1-abc123");
    }
}
