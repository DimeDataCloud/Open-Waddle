//! Desktop sign-in: OAuth 2.0 authorization code flow with PKCE (S256), a
//! random `state`, and a one-shot listener on 127.0.0.1 for Google's redirect.

use anyhow::{bail, Context};
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::{Endpoints, OAuthClient};

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("random bytes");
    b64url(&bytes)
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        Self::from_verifier(random(32))
    }

    pub fn from_verifier(verifier: String) -> Self {
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

pub fn authorize_url(ep: &Endpoints, client_id: &str, redirect_uri: &str, scopes: &[&str], state: &str, challenge: &str) -> String {
    let params = [
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", &scopes.join(" ")),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        // A refresh token, so Waddle stays signed in.
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("include_granted_scopes", "true"),
    ];
    reqwest::Url::parse_with_params(&ep.auth, &params).map(|u| u.to_string()).unwrap_or_default()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default = "an_hour")]
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

fn an_hour() -> u64 {
    3600
}

async fn token_request(http: &reqwest::Client, ep: &Endpoints, client: &OAuthClient, mut form: Vec<(&str, String)>) -> anyhow::Result<Tokens> {
    form.push(("client_id", client.id.clone()));
    if let Some(secret) = client.secret.as_ref().filter(|s| !s.is_empty()) {
        form.push(("client_secret", secret.clone()));
    }
    let resp = http.post(&ep.token).form(&form).send().await.context("reaching Google's sign-in server")?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        let msg = super::api_error(status.as_u16(), &body);
        if body.contains("invalid_grant") {
            bail!("Google sign-in has expired or was revoked; reconnect Google in Settings ({msg})");
        }
        bail!(msg);
    }
    Ok(serde_json::from_str(&body)?)
}

pub async fn exchange(http: &reqwest::Client, ep: &Endpoints, client: &OAuthClient, code: &str, verifier: &str, redirect_uri: &str) -> anyhow::Result<Tokens> {
    token_request(
        http,
        ep,
        client,
        vec![
            ("grant_type", "authorization_code".into()),
            ("code", code.into()),
            ("code_verifier", verifier.into()),
            ("redirect_uri", redirect_uri.into()),
        ],
    )
    .await
}

pub async fn refresh(http: &reqwest::Client, ep: &Endpoints, client: &OAuthClient, refresh_token: &str) -> anyhow::Result<Tokens> {
    token_request(http, ep, client, vec![("grant_type", "refresh_token".into()), ("refresh_token", refresh_token.into())]).await
}

/// Tells Google to forget the grant (on Disconnect). Best effort.
pub async fn revoke(ep: &Endpoints, token: &str) {
    let _ = super::http_client().post(&ep.revoke).form(&[("token", token)]).send().await;
}

/// Runs the browser sign-in. `open` gets the consent page URL (the app opens it in the
/// default browser); the result is Google's tokens, including the refresh token.
pub async fn sign_in(
    ep: &Endpoints,
    client: &OAuthClient,
    scopes: &[&str],
    open: impl FnOnce(&str),
    cancel: &CancellationToken,
    timeout: Duration,
) -> anyhow::Result<Tokens> {
    anyhow::ensure!(!client.id.trim().is_empty(), "add your Google OAuth client ID in Settings first");
    let listener = TcpListener::bind("127.0.0.1:0").await.context("opening a local port for the sign-in")?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let pkce = Pkce::new();
    let state = random(16);
    open(&authorize_url(ep, &client.id, &redirect, scopes, &state, &pkce.challenge));
    let code = tokio::select! {
        r = wait_for_code(&listener, &state) => r?,
        _ = cancel.cancelled() => bail!("sign-in cancelled"),
        _ = tokio::time::sleep(timeout) => bail!("sign-in timed out; try Connect again"),
    };
    let tokens = exchange(&super::http_client(), ep, client, &code, &pkce.verifier, &redirect).await?;
    if tokens.refresh_token.is_none() {
        bail!("Google didn't send a refresh token; remove Waddle's access at myaccount.google.com/permissions and connect again");
    }
    Ok(tokens)
}

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Waddle</title><body style=\"font:16px system-ui;text-align:center;padding:4em\">";

async fn reply(sock: &mut tokio::net::TcpStream, status: &str, text: &str) {
    let body = format!("{PAGE}<p>{text}</p></body>");
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(body.as_bytes()).await;
    let _ = sock.shutdown().await;
}

/// Waits for the browser to come back with `?code=…&state=…`. Other requests (a favicon) get a 404.
async fn wait_for_code(listener: &TcpListener, state: &str) -> anyhow::Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let mut n = 0;
        while n < buf.len() {
            let read = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf[n..])).await.unwrap_or(Ok(0))?;
            if read == 0 {
                break;
            }
            n += read;
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
        let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))?;
        let param = |k: &str| url.query_pairs().find(|(name, _)| name == k).map(|(_, v)| v.into_owned());
        if let Some(err) = param("error") {
            reply(&mut sock, "200 OK", "Sign-in was cancelled. You can close this tab.").await;
            bail!("Google sign-in was cancelled ({err})");
        }
        let Some(code) = param("code") else {
            reply(&mut sock, "404 Not Found", "Not found.").await;
            continue;
        };
        if param("state").as_deref() != Some(state) {
            reply(&mut sock, "400 Bad Request", "This sign-in link doesn't match. Start again from Waddle's Settings.").await;
            bail!("sign-in state didn't match; start again");
        }
        reply(&mut sock, "200 OK", "🦆 Waddle is connected to Google. You can close this tab.").await;
        return Ok(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let fresh = Pkce::new();
        assert!(fresh.verifier.len() >= 43, "RFC 7636 asks for 43-128 characters");
    }

    #[test]
    fn consent_url_asks_for_offline_access_with_pkce() {
        let url = authorize_url(&Endpoints::google(), "abc.apps.googleusercontent.com", "http://127.0.0.1:5555", super::super::SCOPES, "st", "ch");
        for part in ["client_id=abc.apps.googleusercontent.com", "redirect_uri=http%3A%2F%2F127.0.0.1%3A5555", "code_challenge=ch", "code_challenge_method=S256", "access_type=offline", "state=st", "gmail.modify"] {
            assert!(url.contains(part), "{part} missing from {url}");
        }
    }
}
