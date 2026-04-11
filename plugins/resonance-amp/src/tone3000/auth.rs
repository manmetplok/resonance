//! OAuth 2.0 authorization-code + PKCE flow for Tone3000.
//!
//! Native-app flow per RFC 8252: we spin up a one-shot HTTP listener on
//! `127.0.0.1:REDIRECT_PORT`, open the authorize URL in the system
//! browser, wait for the redirect with `?code=...&state=...`, then
//! POST to the token endpoint to exchange the code for tokens.
//!
//! Tokens land in `~/.config/resonance/tone3000.json`. On subsequent
//! launches we load them from there and refresh on 401 rather than
//! re-running the browser dance.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use rand::Rng;
use sha2::{Digest, Sha256};

use super::types::{StoredTokens, TokenResponse};
use super::{API_BASE, REDIRECT_PORT, TONE3000_CLIENT_ID};

const TOKEN_FILE: &str = "resonance/tone3000.json";

/// Location of the persisted token blob. `None` on platforms without
/// an XDG config dir.
pub fn token_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(TOKEN_FILE))
}

pub fn load_tokens() -> Option<StoredTokens> {
    let path = token_path()?;
    let bytes = fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_tokens(tokens: &StoredTokens) -> Result<(), String> {
    let path = token_path().ok_or_else(|| "no config dir".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(tokens).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn clear_tokens() {
    if let Some(path) = token_path() {
        let _ = fs::remove_file(path);
    }
}

/// Generate a PKCE verifier + S256 challenge pair. The verifier is 64
/// random URL-safe bytes (well above the RFC 7636 minimum of 43).
fn pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 64];
    rand::thread_rng().fill(&mut bytes[..]);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill(&mut bytes[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Run the full authorize → redirect → exchange dance. Blocks until the
/// user either approves in the browser or [`timeout`] elapses.
pub fn run_authorization_flow(timeout: Duration) -> Result<StoredTokens, String> {
    if TONE3000_CLIENT_ID.is_empty() {
        return Err(
            "Tone3000 client_id is not configured — edit tone3000/mod.rs".to_string(),
        );
    }

    // Tone3000 normalises the stored redirect URI to `localhost` with a
    // trailing slash regardless of what you register, and then compares
    // byte-for-byte on token exchange. Send exactly that form here —
    // the loopback listener still binds 127.0.0.1 since `localhost`
    // resolves there on Linux.
    let redirect_uri = format!("http://localhost:{REDIRECT_PORT}/");
    let (verifier, challenge) = pkce_pair();
    let state = random_state();

    debug_log(&format!(
        "=== tone3000 oauth flow ===\n\
         redirect_uri: {redirect_uri}\n\
         client_id: {TONE3000_CLIENT_ID}\n\
         verifier ({} chars): {verifier}\n\
         challenge ({} chars): {challenge}\n\
         state: {state}\n",
        verifier.len(),
        challenge.len()
    ));

    // Bind the loopback listener *before* opening the browser so the
    // redirect can never race us.
    let server = tiny_http::Server::http(format!("127.0.0.1:{REDIRECT_PORT}"))
        .map_err(|e| format!("bind 127.0.0.1:{REDIRECT_PORT}: {e}"))?;

    let authorize_url = build_authorize_url(&redirect_uri, &challenge, &state);
    debug_log(&format!("authorize_url: {authorize_url}\n"));
    webbrowser::open(&authorize_url)
        .map_err(|e| format!("open browser: {e}"))?;

    let (code, returned_state) = wait_for_redirect(&server, timeout)?;
    debug_log(&format!(
        "callback code ({} chars): {code}\nreturned_state: {returned_state}\n",
        code.len()
    ));
    if returned_state != state {
        return Err("OAuth state mismatch — possible CSRF".to_string());
    }

    let token = exchange_code_for_token(&code, &verifier, &redirect_uri)?;
    let stored = StoredTokens::from_response(token);
    save_tokens(&stored)?;
    Ok(stored)
}

/// Append to `/tmp/tone3000-auth.log` for debugging the OAuth flow.
/// Temporary — remove once the flow is working reliably.
fn debug_log(msg: &str) {
    use std::io::Write as _;
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/tone3000-auth.log")
    {
        let _ = f.write_all(msg.as_bytes());
    }
}

fn build_authorize_url(redirect_uri: &str, challenge: &str, state: &str) -> String {
    // Hand-build the query string so we don't pull in url::Url just for
    // five parameters. All values are either our own constants or fresh
    // base64url — no characters that would need escaping beyond `:` and
    // `/` in the redirect, which we percent-encode by hand.
    let encoded_redirect = percent_encode(redirect_uri);
    format!(
        "{API_BASE}/api/v1/oauth/authorize?client_id={TONE3000_CLIENT_ID}\
         &redirect_uri={encoded_redirect}\
         &response_type=code\
         &code_challenge={challenge}\
         &code_challenge_method=S256\
         &state={state}\
         &gears=amp\
         &platform=nam"
    )
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

fn wait_for_redirect(
    server: &tiny_http::Server,
    timeout: Duration,
) -> Result<(String, String), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(|| "OAuth flow timed out".to_string())?;

        let req = match server.recv_timeout(remaining) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => return Err(format!("oauth redirect recv: {e}")),
        };

        // Parse `?code=...&state=...` from the request line.
        let url = req.url().to_string();
        let (code, state, error) = parse_callback(&url);

        let body = if error.is_some() || code.is_none() {
            "Authorization failed. You can close this tab and try again."
        } else {
            "Authorized. You can close this tab and return to Resonance."
        };
        let mut response = tiny_http::Response::from_string(body);
        response.add_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..])
                .unwrap(),
        );
        let _ = req.respond(response);

        if let Some(e) = error {
            return Err(format!("authorization denied: {e}"));
        }
        match (code, state) {
            (Some(c), Some(s)) => return Ok((c, s)),
            _ => return Err("OAuth redirect missing code/state".to_string()),
        }
    }
}

fn parse_callback(url: &str) -> (Option<String>, Option<String>, Option<String>) {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let decoded = percent_decode(v);
        match k {
            "code" => code = Some(decoded),
            "state" => state = Some(decoded),
            "error" => error = Some(decoded),
            _ => {}
        }
    }
    (code, state, error)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn exchange_code_for_token(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, String> {
    debug_log(&format!(
        "POST /oauth/token form fields:\n  \
         grant_type=authorization_code\n  \
         code={code}\n  \
         code_verifier={verifier}\n  \
         redirect_uri={redirect_uri}\n  \
         client_id={TONE3000_CLIENT_ID}\n"
    ));
    // `send_form` sets Content-Type itself; don't pre-set it or ureq
    // may double up the header. Error body is explicitly surfaced so
    // a 400 from the server tells us *why* instead of just the status.
    let result = ureq::post(&format!("{API_BASE}/api/v1/oauth/token")).send_form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect_uri),
        ("client_id", TONE3000_CLIENT_ID),
    ]);
    match result {
        Ok(resp) => {
            let body = resp.into_string().unwrap_or_default();
            debug_log(&format!("token response body: {body}\n"));
            serde_json::from_str::<TokenResponse>(&body)
                .map_err(|e| format!("token parse: {e} / body: {body}"))
        }
        Err(e) => {
            let msg = describe_ureq_error(e);
            debug_log(&format!("token exchange error: {msg}\n"));
            Err(msg)
        }
    }
}

pub fn refresh_tokens(refresh_token: &str) -> Result<StoredTokens, String> {
    let resp = ureq::post(&format!("{API_BASE}/api/v1/oauth/token"))
        .send_form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", TONE3000_CLIENT_ID),
        ])
        .map_err(describe_ureq_error)?;
    let token: TokenResponse = resp
        .into_json()
        .map_err(|e| format!("refresh parse: {e}"))?;
    let stored = StoredTokens::from_response(token);
    save_tokens(&stored)?;
    Ok(stored)
}

/// Unpack a `ureq::Error` into a human-readable string that *includes
/// the server's response body* when it's an HTTP status error. Without
/// this, a 400 from `/oauth/token` shows up in the UI as just "HTTP
/// 400" with no explanation, which is useless for debugging.
fn describe_ureq_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            // Cap the body at a sane length so a runaway HTML error
            // page doesn't blow up our status pill.
            let trimmed: String = body.chars().take(400).collect();
            format!("HTTP {code}: {trimmed}")
        }
        ureq::Error::Transport(t) => format!("transport: {t}"),
    }
}

/// Write helper used by the worker to tee the downloaded model into a
/// file while reporting progress. Exposed here so the auth module stays
/// the owner of all filesystem writes under the config dir.
pub fn write_all_to(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut f = fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    f.write_all(bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}
