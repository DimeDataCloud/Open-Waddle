//! Google Drive (read-only): find files, and read them as text. Google Docs and
//! Slides export as text, Sheets as CSV; other files are downloaded and read
//! like local documents.

use serde_json::Value;

use super::Google;

/// The largest file read from Drive.
const MAX_DOWNLOAD: usize = 15 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub modified: String,
    pub link: String,
}

fn file(v: &Value) -> DriveFile {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    DriveFile { id: s("id"), name: s("name"), mime: s("mimeType"), modified: s("modifiedTime"), link: s("webViewLink") }
}

/// A Drive query matching names or contents, with quotes escaped.
pub fn search_query(words: &str) -> String {
    let w = words.trim().replace('\\', "\\\\").replace('\'', "\\'");
    format!("(name contains '{w}' or fullText contains '{w}') and trashed = false")
}

/// A short kind for a MIME type.
pub fn kind(mime: &str) -> &'static str {
    match mime {
        "application/vnd.google-apps.document" => "Google Doc",
        "application/vnd.google-apps.spreadsheet" => "Google Sheet",
        "application/vnd.google-apps.presentation" => "Google Slides",
        "application/vnd.google-apps.folder" => "folder",
        "application/pdf" => "PDF",
        m if m.contains("wordprocessingml") => "Word",
        m if m.contains("spreadsheetml") => "Excel",
        m if m.contains("presentationml") => "PowerPoint",
        m if m.starts_with("image/") => "image",
        m if m.starts_with("text/") => "text",
        _ => "file",
    }
}

impl Google {
    pub async fn drive_search(&self, words: &str, max: u32) -> anyhow::Result<Vec<DriveFile>> {
        anyhow::ensure!(!words.trim().is_empty(), "`query` is required");
        let r = self
            .get(
                &format!("{}/files", self.endpoints.drive),
                &[
                    ("q", search_query(words)),
                    ("pageSize", max.clamp(1, 25).to_string()),
                    ("fields", "files(id,name,mimeType,modifiedTime,webViewLink)".into()),
                    ("supportsAllDrives", "true".into()),
                    ("includeItemsFromAllDrives", "true".into()),
                ],
            )
            .await?;
        Ok(r["files"].as_array().into_iter().flatten().map(file).collect())
    }

    /// A Drive file as text: (its details, the text).
    pub async fn drive_read(&self, id: &str) -> anyhow::Result<(DriveFile, String)> {
        let id = id.trim();
        anyhow::ensure!(!id.is_empty(), "`id` is required (from drive_search)");
        let base = format!("{}/files/{id}", self.endpoints.drive);
        let meta = self.get(&base, &[("fields", "id,name,mimeType,modifiedTime,webViewLink,size".into()), ("supportsAllDrives", "true".into())]).await?;
        let f = file(&meta);
        let export = match f.mime.as_str() {
            "application/vnd.google-apps.document" | "application/vnd.google-apps.presentation" => Some("text/plain"),
            "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
            "application/vnd.google-apps.folder" => anyhow::bail!("\"{}\" is a folder; search inside it with drive_search", f.name),
            m if m.starts_with("application/vnd.google-apps.") => anyhow::bail!("\"{}\" is a {} and can't be read as text", f.name, m.trim_start_matches("application/vnd.google-apps.")),
            _ => None,
        };
        let text = match export {
            Some(mime) => {
                let bytes = self.call_raw(reqwest::Method::GET, &format!("{base}/export"), &[("mimeType", mime.into())], None, MAX_DOWNLOAD).await?;
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if mime == "text/csv" { crate::tools::docs::read_bytes("sheet.csv", text.as_bytes(), None)? } else { crate::tools::docs::read_bytes("doc.txt", text.as_bytes(), None)? }
            }
            None => {
                let bytes = self.call_raw(reqwest::Method::GET, &base, &[("alt", "media".into()), ("supportsAllDrives", "true".into())], None, MAX_DOWNLOAD).await?;
                crate::tools::docs::read_bytes(&f.name, &bytes, None)?
            }
        };
        Ok((f, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_escape_quotes() {
        assert_eq!(search_query("Ana's budget"), "(name contains 'Ana\\'s budget' or fullText contains 'Ana\\'s budget') and trashed = false");
        assert_eq!(kind("application/vnd.google-apps.spreadsheet"), "Google Sheet");
        assert_eq!(kind("application/vnd.openxmlformats-officedocument.wordprocessingml.document"), "Word");
    }
}
