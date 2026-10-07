//! The server index: fetch, parse leniently, and cache beside a library.
//!
//! The document is one `index.json` with a list per content kind
//! (`drumkits`, `voicebanks`, `vocoders`); each owner deserializes the
//! lists it knows and ignores the rest. Entries keep everything past their
//! identity optional, read through [`lenient`], so a field of the wrong
//! type reads as absent rather than failing the whole index.

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use super::{DownloadError, INDEX_CACHE_FILE};

/// `T` if the value deserializes as one, else `None` — one odd field does
/// not cost the user the whole index. Use as
/// `#[serde(default, deserialize_with = "lenient")]` on an `Option<T>`.
pub fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// A list of strings, keeping the strings and dropping anything else; a
/// non-list reads as empty.
pub fn lenient_tags<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    })
}

/// Parse an index document.
pub fn parse_index<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DownloadError> {
    serde_json::from_slice(bytes).map_err(|e| DownloadError::Failed(format!("parse index: {e}")))
}

/// Fetch and parse the index at `url`. The agent's own budget bounds the
/// call (see [`super::http::agent`]).
pub fn fetch_index<T: DeserializeOwned>(
    agent: &ureq::Agent,
    url: &str,
) -> Result<T, DownloadError> {
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| DownloadError::Failed(format!("fetch index: {e}")))?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(DownloadError::Failed(format!(
            "fetch index: the server answered {status}"
        )));
    }
    let body = resp
        .body_mut()
        .read_to_vec()
        .map_err(|e| DownloadError::Failed(format!("read index: {e}")))?;
    parse_index(&body)
}

/// The index cached under `root`, if one was ever fetched (and still
/// parses as `T`).
pub fn read_index_cache<T: DeserializeOwned>(root: &Path) -> Option<T> {
    let bytes = std::fs::read(root.join(INDEX_CACHE_FILE)).ok()?;
    parse_index(&bytes).ok()
}

/// Cache `index` under `root`. Does nothing when `root` does not exist yet
/// (nothing is installed: the library is not created just for this);
/// failures are logged, never fatal.
pub fn write_index_cache<T: Serialize>(root: &Path, index: &T) {
    if !root.is_dir() {
        return;
    }
    let path = root.join(INDEX_CACHE_FILE);
    match serde_json::to_vec_pretty(index) {
        Ok(bytes) => {
            if let Err(e) = crate::atomic_file::atomic_write(&path, &bytes) {
                tracing::warn!("plok.org index cache {}: {e}", path.display());
            }
        }
        Err(e) => tracing::warn!("plok.org index cache: serialize: {e}"),
    }
}
