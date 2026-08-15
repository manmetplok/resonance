//! Thin blocking HTTP wrapper over `ureq` for the Tone3000 REST API.
//!
//! All calls take the current access token explicitly so the worker
//! can swap it out after a refresh without rebuilding the client.
//!
//! Everything that decides *what* to ask for — the query string, the
//! architecture filter, how several responses merge into one page — is
//! a pure function down here so it can be unit-tested without a
//! network (see `tests/tone3000_browser.rs`).

use std::io::Read as _;

use super::types::{Model, PaginatedResponse, Tone};
use super::API_BASE;

/// Gear filter, underscore-separated per the tone3000 spec.
///
/// Deliberate choice, recorded rather than lifted (ba todo #1315): this
/// plugin plays amp/rig captures, so it asks for bare amp profiles plus
/// bundled amp+cab+mic snapshots. (`full-rig` is the deprecated alias
/// the API normalises to `amp-cab`.) The API also offers `cab`, `pedal`,
/// `outboard`, `space` and `experimental`; exposing those is a product
/// decision about what the amp plugin is for, not a bug fix, and it
/// would overlap resonance-ir's territory.
const GEARS_AMP: &str = "amp_amp-cab";

/// `page_size` for `/tones/search`. 25 is the documented maximum; the
/// server default is 10.
const SEARCH_PAGE_SIZE: u32 = 25;

/// `page_size` for `/models`. The documented maximum is 300; a tone with
/// more than 100 models does not exist in practice, and this keeps the
/// response small.
const MODELS_PAGE_SIZE: u32 = 100;

/// Which NAM architectures the browser asks for.
///
/// The API's `architecture` parameter takes exactly one of `1`, `2` or
/// `custom` — it is *not* a multi-value filter like `gears`. Omitting it
/// is the legacy default and returns A1 + custom (it predates A2, so it
/// excludes it). Covering more than one architecture therefore means
/// issuing more than one request and merging, which is what
/// [`Tone3000Client::search_tones`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArchitectureFilter {
    /// Everything the site has: A1, A2 and custom. The default — the
    /// engine runs A1 reference-exact and A2 alike, so there is no
    /// reason to hide either from the browser.
    #[default]
    All,
    /// Architecture 1 only (the original NAM WaveNet/LSTM format).
    A1,
    /// Architecture 2 only.
    A2,
    /// Community/custom architectures.
    Custom,
}

impl ArchitectureFilter {
    /// Every variant, in the order the UI dropdown lists them.
    pub const ALL: &'static [ArchitectureFilter] = &[
        ArchitectureFilter::All,
        ArchitectureFilter::A1,
        ArchitectureFilter::A2,
        ArchitectureFilter::Custom,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ArchitectureFilter::All => "All architectures",
            ArchitectureFilter::A1 => "A1 only",
            ArchitectureFilter::A2 => "A2 only",
            ArchitectureFilter::Custom => "Custom only",
        }
    }

    /// The `architecture` query values that together cover this filter.
    /// `None` means "send no `architecture` parameter", which the API
    /// answers with A1 + custom.
    ///
    /// So [`ArchitectureFilter::All`] is two requests — the legacy
    /// A1+custom default, then A2 — rather than three.
    pub fn query_values(&self) -> &'static [Option<&'static str>] {
        match self {
            ArchitectureFilter::All => &[None, Some("2")],
            ArchitectureFilter::A1 => &[Some("1")],
            ArchitectureFilter::A2 => &[Some("2")],
            ArchitectureFilter::Custom => &[Some("custom")],
        }
    }
}

/// One page of search results, already merged across the sub-requests a
/// multi-architecture filter needs.
#[derive(Debug, Clone, Default)]
pub struct SearchPage {
    pub tones: Vec<Tone>,
    /// 1-based page index this data came from.
    pub page: u32,
    /// Total matching tones as reported by the server, summed across
    /// sub-requests. `None` if the server reported nothing.
    pub total: Option<u32>,
    /// Highest `total_pages` across sub-requests — i.e. how many pages
    /// must be walked to see everything.
    pub total_pages: Option<u32>,
}

/// Build the `/tones/search` query string. Pure so the parameter set is
/// testable: in particular that `architecture` is *absent* rather than
/// pinned when the caller passes `None`.
pub fn search_query_params(
    query: &str,
    sort: &str,
    architecture: Option<&str>,
    page: u32,
) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("query", query.to_string()),
        ("sort", sort.to_string()),
        ("gears", GEARS_AMP.to_string()),
        // `format` supersedes the deprecated `platform` alias.
        ("format", "nam".to_string()),
        ("page", page.max(1).to_string()),
        ("page_size", SEARCH_PAGE_SIZE.to_string()),
    ];
    if let Some(arch) = architecture {
        params.push(("architecture", arch.to_string()));
    }
    params
}

/// Build the `/models` query string. Same reasoning as
/// [`search_query_params`].
pub fn models_query_params(tone_id: i64, architecture: Option<&str>) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("tone_id", tone_id.to_string()),
        ("page_size", MODELS_PAGE_SIZE.to_string()),
    ];
    if let Some(arch) = architecture {
        params.push(("architecture", arch.to_string()));
    }
    params
}

/// Merge the sub-responses of one logical page into a single
/// [`SearchPage`], keeping first-seen order and dropping tones that
/// appear in more than one sub-response.
///
/// Totals are summed: the sub-queries partition the catalogue by
/// architecture, so a tone counted twice would need models of two
/// architectures at once. The dedupe below keeps the *rendered* list
/// honest either way.
pub fn merge_search_pages(pages: Vec<PaginatedResponse<Tone>>, page: u32) -> SearchPage {
    let mut tones: Vec<Tone> = Vec::new();
    let mut seen: Vec<i64> = Vec::new();
    let mut total: Option<u32> = None;
    let mut total_pages: Option<u32> = None;

    for resp in pages {
        if let Some(t) = resp.total {
            total = Some(total.unwrap_or(0) + t);
        }
        if let Some(tp) = resp.total_pages {
            total_pages = Some(total_pages.unwrap_or(0).max(tp));
        }
        for tone in resp.data {
            if seen.contains(&tone.id) {
                continue;
            }
            seen.push(tone.id);
            tones.push(tone);
        }
    }

    SearchPage {
        tones,
        page: page.max(1),
        total,
        total_pages,
    }
}

/// Same merge for `/models`, which the browser never paginates.
pub fn merge_model_pages(pages: Vec<PaginatedResponse<Model>>) -> Vec<Model> {
    let mut models: Vec<Model> = Vec::new();
    let mut seen: Vec<i64> = Vec::new();
    for resp in pages {
        for model in resp.data {
            if seen.contains(&model.id) {
                continue;
            }
            seen.push(model.id);
            models.push(model);
        }
    }
    models
}

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
        // ureq 3 routes HTTP status errors through Error::StatusCode
        // by default, discarding the response body. We turn that off
        // so we can read the body for the 401 special-case and for
        // descriptive error messages.
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        let agent: ureq::Agent = config.into();
        Self { agent }
    }

    /// Fetch one page of tones for `architecture`, merging the requests
    /// the filter needs. Any sub-request failing fails the whole page —
    /// half a page of results with no indication would be worse than an
    /// error the user can retry.
    pub fn search_tones(
        &self,
        token: &str,
        query: &str,
        sort: &str,
        architecture: ArchitectureFilter,
        page: u32,
    ) -> Result<SearchPage, ClientError> {
        let mut responses = Vec::new();
        for arch in architecture.query_values() {
            responses.push(self.search_tones_once(token, query, sort, *arch, page)?);
        }
        Ok(merge_search_pages(responses, page))
    }

    fn search_tones_once(
        &self,
        token: &str,
        query: &str,
        sort: &str,
        architecture: Option<&str>,
        page: u32,
    ) -> Result<PaginatedResponse<Tone>, ClientError> {
        let url = format!("{API_BASE}/api/v1/tones/search");
        let mut req = self
            .agent
            .get(&url)
            .header("Authorization", format!("Bearer {token}"));
        for (k, v) in search_query_params(query, sort, architecture, page) {
            req = req.query(k, v);
        }
        let mut resp = req.call()?;
        check_status(&mut resp)?;
        resp.body_mut()
            .read_json()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub fn list_models(
        &self,
        token: &str,
        tone_id: i64,
        architecture: ArchitectureFilter,
    ) -> Result<Vec<Model>, ClientError> {
        let mut responses = Vec::new();
        for arch in architecture.query_values() {
            responses.push(self.list_models_once(token, tone_id, *arch)?);
        }
        Ok(merge_model_pages(responses))
    }

    fn list_models_once(
        &self,
        token: &str,
        tone_id: i64,
        architecture: Option<&str>,
    ) -> Result<PaginatedResponse<Model>, ClientError> {
        let url = format!("{API_BASE}/api/v1/models");
        let mut req = self
            .agent
            .get(&url)
            .header("Authorization", format!("Bearer {token}"));
        for (k, v) in models_query_params(tone_id, architecture) {
            req = req.query(k, v);
        }
        let mut resp = req.call()?;
        check_status(&mut resp)?;
        resp.body_mut()
            .read_json()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    /// Fetch a model's bytes. Returns the full body in memory — NAM
    /// profiles are typically 1–50 MB, comfortably fine for a single
    /// allocation. Streaming to disk would be nicer but adds a pile of
    /// state machine for negligible benefit at these sizes.
    pub fn download_model(&self, token: &str, model_url: &str) -> Result<Vec<u8>, ClientError> {
        let mut resp = self
            .agent
            .get(model_url)
            .header("Authorization", format!("Bearer {token}"))
            .call()?;
        check_status(&mut resp)?;

        let len_hint: usize = resp
            .headers()
            .get("Content-Length")
            .and_then(|s| s.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // Cap at 200 MB to avoid a bug in the server sending us a
        // runaway Content-Length turning into an OOM.
        const MAX_BYTES: usize = 200 * 1024 * 1024;
        let mut buf = Vec::with_capacity(len_hint.min(MAX_BYTES));
        resp.into_body()
            .into_reader()
            .take(MAX_BYTES as u64)
            .read_to_end(&mut buf)
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        Ok(buf)
    }
}

fn check_status(resp: &mut ureq::http::Response<ureq::Body>) -> Result<(), ClientError> {
    let code = resp.status().as_u16();
    if code == 401 {
        return Err(ClientError::Unauthorized);
    }
    if !(200..300).contains(&code) {
        let msg = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(ClientError::Http(format!("HTTP {code}: {msg}")));
    }
    Ok(())
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
            ureq::Error::StatusCode(401) => ClientError::Unauthorized,
            other => ClientError::Http(other.to_string()),
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
