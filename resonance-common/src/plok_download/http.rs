//! The HTTP client: one `ureq` agent per worker, with the timeouts every
//! plok.org transfer uses.

use std::time::Duration;

use super::{read_timeout::ReadTimeout, CONNECT_TIMEOUT, READ_TIMEOUT};

/// How an owner's agent talks to the network. Tests point it at a local
/// server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpConfig {
    /// Refuse plain-HTTP URLs (redirects included). On for the real
    /// server; the tests' loopback server speaks HTTP and turns it off.
    pub https_only: bool,
    /// The per-read stall limit, [`READ_TIMEOUT`] by default. Tests
    /// shorten it to see a stalled transfer fail in milliseconds.
    pub read_timeout: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            https_only: true,
            read_timeout: READ_TIMEOUT,
        }
    }
}

/// An agent with the per-read stall limit; `global` caps a whole call (the
/// index fetches), `None` leaves a multi-GiB download unbounded.
pub fn agent(config: &HttpConfig, global: Option<Duration>) -> ureq::Agent {
    let read_timeout = config.read_timeout;
    let agent_config = ureq::Agent::config_builder()
        .https_only(config.https_only)
        // Statuses are classified by hand: a 416 carries the information
        // a resume needs, and a 5xx keeps the `.part`.
        .http_status_as_error(false)
        .timeout_global(global)
        .timeout_resolve(Some(CONNECT_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_send_request(Some(read_timeout))
        .timeout_recv_response(Some(read_timeout))
        // No total body budget for a download. A stalled body is caught
        // per read by `ReadTimeout` instead.
        .build();
    ureq::Agent::with_parts(
        agent_config,
        ReadTimeout::new(read_timeout),
        ureq::unversioned::resolver::DefaultResolver::default(),
    )
}

/// A response header's value, trimmed.
pub(super) fn header(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
}

/// `Content-Range: bytes <start>-<end>/<total>` or `bytes */<total>`:
/// (start, total), each `None` when absent or `*`.
pub(super) fn content_range(resp: &ureq::http::Response<ureq::Body>) -> (Option<u64>, Option<u64>) {
    let Some(v) = header(resp, "Content-Range") else {
        return (None, None);
    };
    let Some(rest) = v.strip_prefix("bytes ") else {
        return (None, None);
    };
    let (range, total) = rest.split_once('/').unwrap_or((rest, "*"));
    let start = range
        .split_once('-')
        .and_then(|(s, _)| s.trim().parse().ok());
    (start, total.trim().parse().ok())
}
