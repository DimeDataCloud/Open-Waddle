//! Google Contacts (read-only): saved contacts and "other contacts" (people
//! the user has emailed), searched by name or address.

use serde_json::Value;
use std::sync::atomic::Ordering;

use super::Google;

#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    pub name: String,
    pub email: String,
}

fn contacts(v: &Value) -> Vec<Contact> {
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|r| {
            let p = &r["person"];
            let name = p["names"][0]["displayName"].as_str().unwrap_or("").to_string();
            p["emailAddresses"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e["value"].as_str())
                .map(|email| Contact { name: name.clone(), email: email.to_string() })
                .collect::<Vec<_>>()
        })
        .collect()
}

impl Google {
    pub async fn contacts_find(&self, query: &str) -> anyhow::Result<Vec<Contact>> {
        let query = query.trim();
        anyhow::ensure!(!query.is_empty(), "`name` is required");
        let saved = format!("{}/people:searchContacts", self.endpoints.people);
        let other = format!("{}/otherContacts:search", self.endpoints.people);
        if !self.contacts_warm.swap(true, Ordering::SeqCst) {
            // Google's search cache is only filled by a first, empty query.
            let mask = ("readMask", "names,emailAddresses".to_string());
            let _ = futures_util::future::join(
                self.get(&saved, &[("query", String::new()), mask.clone()]),
                self.get(&other, &[("query", String::new()), ("readMask", "names,emailAddresses".into())]),
            )
            .await;
        }
        let (a, b) = futures_util::future::join(
            self.get(&saved, &[("query", query.to_string()), ("readMask", "names,emailAddresses".into()), ("pageSize", "10".into())]),
            self.get(&other, &[("query", query.to_string()), ("readMask", "names,emailAddresses".into()), ("pageSize", "10".into())]),
        )
        .await;
        let mut out: Vec<Contact> = vec![];
        // Saved contacts first; other contacts can't make a search fail.
        for c in contacts(&a?).into_iter().chain(b.map(|v| contacts(&v)).unwrap_or_default()) {
            if !out.iter().any(|x| x.email.eq_ignore_ascii_case(&c.email)) {
                out.push(c);
            }
        }
        out.truncate(10);
        Ok(out)
    }
}
