//! Learned skills: short instructions Waddle writes for itself (with the
//! user's approval) and reads back into its system prompt on every task.
//! This is how Waddle improves itself without code changes. Skills live
//! outside the workspace so file tools can't rewrite them behind the gate.

use anyhow::{bail, Context};
use std::path::{Path, PathBuf};

const MAX_SKILL_BYTES: usize = 4000;
const MAX_TOTAL_BYTES: usize = 12_000;

pub struct SkillStore {
    dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub body: String,
}

/// Lowercase letters, digits and dashes only, so names map safely to files.
pub fn normalize_name(name: &str) -> anyhow::Result<String> {
    let n: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if n.is_empty() || n.len() > 60 {
        bail!("skill names need 1-60 letters or digits");
    }
    Ok(n)
}

impl SkillStore {
    pub fn new(dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir.as_ref()).with_context(|| format!("creating {}", dir.as_ref().display()))?;
        Ok(Self { dir: dir.as_ref().to_path_buf() })
    }

    pub fn list(&self) -> Vec<Skill> {
        let mut out: Vec<Skill> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
            .filter_map(|e| {
                let name = e.path().file_stem()?.to_string_lossy().into_owned();
                let body = std::fs::read_to_string(e.path()).ok()?;
                Some(Skill { name, body })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn save(&self, name: &str, body: &str) -> anyhow::Result<String> {
        let name = normalize_name(name)?;
        let body = body.trim();
        if body.is_empty() {
            bail!("a skill needs instructions");
        }
        if body.len() > MAX_SKILL_BYTES {
            bail!("keep skills under {MAX_SKILL_BYTES} bytes");
        }
        let others: usize = self.list().iter().filter(|s| s.name != name).map(|s| s.body.len()).sum();
        if others + body.len() > MAX_TOTAL_BYTES {
            bail!("skill memory is full ({MAX_TOTAL_BYTES} bytes); forget one first");
        }
        let path = self.dir.join(format!("{name}.md"));
        let existed = path.exists();
        std::fs::write(&path, body)?;
        Ok(format!("{} skill \"{name}\".", if existed { "Updated" } else { "Learned" }))
    }

    pub fn forget(&self, name: &str) -> anyhow::Result<String> {
        let name = normalize_name(name)?;
        let path = self.dir.join(format!("{name}.md"));
        if !path.exists() {
            bail!("no skill called \"{name}\"");
        }
        std::fs::remove_file(path)?;
        Ok(format!("Forgot skill \"{name}\"."))
    }

    /// The section appended to the system prompt.
    pub fn prompt_section(&self) -> Option<String> {
        let skills = self.list();
        if skills.is_empty() {
            return None;
        }
        let body: Vec<String> = skills.iter().map(|s| format!("### {}\n{}", s.name, s.body)).collect();
        Some(format!(
            "Skills you have learned (each was approved by the user; follow them when relevant):\n{}",
            body.join("\n\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_forget() {
        let dir = tempfile::tempdir().unwrap();
        let store = SkillStore::new(dir.path()).unwrap();
        assert!(store.prompt_section().is_none());
        assert_eq!(store.save("Open Notepad Fast!", "Use open_app with \"notepad\".").unwrap(), "Learned skill \"open-notepad-fast\".");
        assert!(store.save("open-notepad-fast", "v2").unwrap().starts_with("Updated"));
        assert_eq!(store.list(), vec![Skill { name: "open-notepad-fast".into(), body: "v2".into() }]);
        assert!(store.prompt_section().unwrap().contains("### open-notepad-fast\nv2"));
        store.forget("open-notepad-fast").unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn names_cannot_escape_and_sizes_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        let store = SkillStore::new(dir.path()).unwrap();
        assert_eq!(normalize_name("../../etc/passwd").unwrap(), "etc-passwd");
        assert!(normalize_name("..").is_err());
        assert!(store.save("big", &"x".repeat(MAX_SKILL_BYTES + 1)).is_err());
        for i in 0..3 {
            store.save(&format!("s{i}"), &"x".repeat(MAX_SKILL_BYTES)).unwrap();
        }
        assert!(store.save("one-more", "x").is_err());
    }
}
