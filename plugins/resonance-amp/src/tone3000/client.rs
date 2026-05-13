//! Thin blocking HTTP wrapper over `ureq` for the Tone3000 REST API.
//!
//! All calls take the current access token explicitly so the worker
//! can swap it out after a refresh without rebuilding the client.

use std::io::Read as _;

use super::types::{Model, PaginatedResponse, Tone};
use super::API_BASE;

pub struct Tone3000Client {
    agent: ureq::Agent,
}

impl Default for Tone3000Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Tone3000Client {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build();
        Self { agent }
    }

    pub fn search_tones(
        &self,
        token: &str,
        query: &str,
        sort: &str,
        page: u32,
    ) -> Result<Vec<Tone>, ClientError> {
        let url = format!("{API_BASE}/api/v1/tones/search");
        let resp = self
            .agent
            .get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .query("query", query)
            .query("sort", sort)
            // Underscore-separated multi-value filter per the tone3000
            // spec. Includes full-rig so bundled amp+cab+mic snapshots
            // show up alongside bare amp profiles.
            .query("gears", "amp_full-rig")
            .query("platform", "nam")
            .query("page", &page.to_string())
            .query("page_size", "25")
            .call()
            .map_err(ClientError::from)?;
        let page: PaginatedResponse<Tone> = resp
            .into_json()
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        Ok(page.data)
    }

    pub fn list_models(&self, token: &str, tone_id: i64) -> Result<Vec<Model>, ClientError> {
        let url = format!("{API_BASE}/api/v1/models");
        let resp = self
            .agent
            .get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .query("tone_id", &tone_id.to_string())
            .query("page_size", "100")
            .call()
            .map_err(ClientError::from)?;
        let page: PaginatedResponse<Model> = resp
            .into_json()
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        Ok(page.data)
    }

    /// Fetch a model's bytes. Returns the full body in memory — NAM
    /// profiles are typically 1–50 MB, comfortably fine for a single
    /// allocation. Streaming to disk would be nicer but adds a pile of
    /// state machine for negligible benefit at these sizes.
    pub fn download_model(&self, token: &str, model_url: &str) -> Result<Vec<u8>, ClientError> {
        let resp = self
            .agent
            .get(model_url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(ClientError::from)?;

        let len_hint: usize = resp
            .header("Content-Length")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Cap at 200 MB to avoid a bug in the server sending us a
        // runaway Content-Length turning into an OOM.
        const MAX_BYTES: usize = 200 * 1024 * 1024;
        let mut buf = Vec::with_capacity(len_hint.min(MAX_BYTES));
        resp.into_reader()
            .take(MAX_BYTES as u64)
            .read_to_end(&mut buf)
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        Ok(buf)
    }
}

#[derive(Debug)]
pub enum ClientError {
    /// The server answered 401 — tokens are stale and need refreshing
    /// before the call can be retried.
    Unauthorized,
    /// Any other HTTP or transport error, stringified for the UI.
    Http(String),
    /// Response body didn't deserialize.
    Parse(String),
}

impl From<ureq::Error> for ClientError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::Status(401, _) => ClientError::Unauthorized,
            ureq::Error::Status(code, resp) => {
                let msg = resp.into_string().unwrap_or_default();
                ClientError::Http(format!("HTTP {code}: {msg}"))
            }
            ureq::Error::Transport(t) => ClientError::Http(t.to_string()),
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Unauthorized => write!(f, "unauthorized (token expired)"),
            ClientError::Http(m) => write!(f, "{m}"),
            ClientError::Parse(m) => write!(f, "response parse: {m}"),
        }
    }
}
