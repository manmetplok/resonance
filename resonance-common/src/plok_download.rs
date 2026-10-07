//! The plok.org download machinery shared by every library that installs
//! from `https://resonance.plok.org/index.json` — drum kits (the drums
//! plugin's worker, drums-plugin-rework.md §4) and voicebanks with their
//! vocoders (the app, vocal-mcp-control-and-quality.md §5).
//!
//! What is here is the transport, not a worker: each owner keeps its own
//! queue, threads, status and install step, and calls into this for
//!
//! - **HTTP** ([`http`]): a `ureq` agent with connect/read timeouts — the
//!   read timeout is per read, so a multi-GiB file takes as long as it
//!   takes but a silent connection fails — and an HTTPS-only switch the
//!   tests' loopback server turns off;
//! - **the index** ([`index`]): fetching and parsing a JSON index with a
//!   total time budget, lenient optional fields (one odd field never costs
//!   the user the whole index), and the on-disk cache an owner keeps beside
//!   its library ([`INDEX_CACHE_FILE`], fresh for [`INDEX_TTL`]);
//! - **a download** ([`transfer::download`]): into a `.part` file under an
//!   advisory lock, resumed with `Range` + `If-Range` from an unowned part
//!   of the same file, refused up front when the disk lacks
//!   [`DOWNLOAD_SPACE_FACTOR`] × the file, hashed (sha256) as it streams,
//!   and verified against the index's hash ([`DownloadError::ChecksumMismatch`]);
//!   the owner's [`transfer::Control`] answers "stop?" every chunk and
//!   receives progress. See [`part`] for the part-file protocol;
//! - **a package install** ([`package::install_package`], feature
//!   `drumkit-zip`): a downloaded zip unpacked into a folder through a
//!   staging directory, a wrapper directory stripped, macOS litter
//!   skipped — for content whose layout is "a folder with a marker file
//!   near the top" (a voicebank's `dsconfig.yaml`, a vocoder's
//!   `vocoder.yaml`). Drum kits keep the kit library's own install
//!   (`drumkit_library::Library::install_zip`), which also maintains
//!   their slot table.
//!
//! Gated behind the `plok-download` feature: a build that never downloads
//! (a headless plugin, a render) does not compile `ureq`.

use std::time::Duration;

pub mod http;
pub mod index;
#[cfg(feature = "drumkit-zip")]
pub mod package;
pub mod part;
mod read_timeout;
pub mod transfer;

pub use http::HttpConfig;
pub use index::{
    fetch_index, lenient, lenient_tags, parse_index, read_index_cache, write_index_cache,
};
pub use part::{new_nonce, part_path, sweep_stale_downloads, PartFile};
pub use transfer::{download, Control, Progress, Stop, Target};

/// The distribution server's index.
pub const INDEX_URL: &str = "https://resonance.plok.org/index.json";

/// How long a read may wait for the next byte before the transfer is
/// abandoned as stalled. Per read, not per transfer: a 5 GiB kit takes as
/// long as it takes, but a connection that goes silent fails after this.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// How long name resolution and connecting may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The whole budget of one index fetch (resolve, connect, headers, body).
pub const INDEX_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a fetched index is fresh: opening a download tab within this
/// of the last fetch does not fetch again (Refresh always does).
pub const INDEX_TTL: Duration = Duration::from_secs(10 * 60);

/// A download needs this much free space per byte of zip: the zip and the
/// extracted content coexist until the zip is deleted.
pub const DOWNLOAD_SPACE_FACTOR: f64 = 2.1;

/// The fetched index, cached under a library root (hidden, so a scan never
/// sees it) for an offline tab.
pub const INDEX_CACHE_FILE: &str = ".plok-index.json";

/// Why a download did not produce a verified file.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The owner cancelled it: its `.part` is gone.
    #[error("cancelled")]
    Cancelled,
    /// Nobody wants it any more (shutdown, the owner let go): its `.part`
    /// stays for a resume.
    #[error("abandoned")]
    Abandoned,
    /// It failed. A network failure keeps the `.part` for a resume; any
    /// other failure removed it.
    #[error("{0}")]
    Failed(String),
    /// The file arrived whole but is not the one the index pins. The part
    /// is removed; nothing was installed. Each owner words this for its
    /// users (a kit "failed verification"; a voicebank's upstream file
    /// "changed").
    #[error("\"{name}\": the download's sha256 is {actual}, the index says {expected}")]
    ChecksumMismatch {
        name: String,
        actual: String,
        expected: String,
    },
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// A panic payload as text (the workers catch a panicking job and report
/// it instead of dying).
pub fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "internal error".to_string())
}

/// Whether an index entry's `id` may name a folder under a library root:
/// one plain path component of ASCII letters, digits, `.`, `_` and `-`,
/// not starting with `.` (no `..`, no hidden folder, no separator, no
/// absolute path). An index is remote input, and the id becomes the
/// folder an install writes and a Delete removes, so anything else is
/// refused rather than sanitised.
pub fn is_safe_id(id: &str) -> bool {
    use std::path::{Component, Path};
    if id.is_empty()
        || id.starts_with('.')
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return false;
    }
    let mut components = Path::new(id).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}
