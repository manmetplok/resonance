//! One download: a file from the index, streamed into a [`PartFile`] with
//! resume, a disk check, progress, and the sha256 verify.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::http::{content_range, header};
use super::part::{io_failure, PartFile};
use super::{hex, DownloadError, DOWNLOAD_SPACE_FACTOR};
use crate::drumkit_library::format_bytes;

/// What to download.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    /// The display name, used in messages and progress.
    pub name: String,
    /// The index entry's file name: what part files are named after, so a
    /// resume finds the part of the same file.
    pub file: String,
    /// Where it is fetched from.
    pub url: String,
    /// The file's exact size, when the index gives it.
    pub bytes: Option<u64>,
    /// Its sha256 (hex), when the index pins it.
    pub sha256: Option<String>,
}

impl Target {
    pub(super) fn sha256_lower(&self) -> Option<String> {
        self.sha256.as_ref().map(|s| s.trim().to_ascii_lowercase())
    }
}

/// Why the owner wants a running download to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The user cancelled it: its `.part` goes.
    Cancelled,
    /// Nobody wants it any more: its `.part` stays for a resume.
    Abandoned,
}

impl From<Stop> for DownloadError {
    fn from(s: Stop) -> Self {
        match s {
            Stop::Cancelled => DownloadError::Cancelled,
            Stop::Abandoned => DownloadError::Abandoned,
        }
    }
}

/// How far a download has got.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Progress {
    pub downloaded_bytes: u64,
    /// 0 when the server did not say.
    pub total_bytes: u64,
    /// Smoothed transfer rate, bytes per second (0 until measured).
    pub bytes_per_sec: f64,
    /// Seconds left at the current rate, once there is one.
    pub eta_secs: Option<f64>,
    /// Bytes kept from an earlier, interrupted transfer (Range resume).
    pub resumed_from: u64,
}

/// The owner's side of a running download: asked every chunk whether to
/// stop, and told how far it got.
pub trait Control {
    /// Why the download must stop now, if it must.
    fn stop(&self) -> Option<Stop>;

    /// Called after every chunk (and once on a resume that finds the part
    /// already whole).
    fn progress(&self, _progress: Progress) {}

    /// Called once the file is whole, before its sha256 is compared.
    fn verifying(&self) {}

    /// Bytes available at `at`; `None` when unknown (no check is made).
    /// The real probe by default; tests replace it.
    fn free_space(&self, at: &Path) -> Option<u64> {
        crate::drumkit_library::free_space(at)
    }
}

/// How a transfer failed.
enum Transfer {
    /// The connection failed or stalled, or the server is (for now)
    /// unwilling: the `.part` is kept for a resume.
    Network(String),
    Other(DownloadError),
}

impl From<DownloadError> for Transfer {
    fn from(f: DownloadError) -> Self {
        Transfer::Other(f)
    }
}

impl From<Stop> for Transfer {
    fn from(s: Stop) -> Self {
        Transfer::Other(s.into())
    }
}

/// A non-success answer: a server error, a timeout or rate limit is worth
/// retrying (the `.part` stays); any other 4xx is a definite no.
fn status_failure(name: &str, code: u16) -> Transfer {
    let msg = format!("download {name}: the server answered {code}");
    if code >= 500 || code == 408 || code == 429 {
        Transfer::Network(msg)
    } else {
        Transfer::Other(DownloadError::Failed(msg))
    }
}

fn request_failure(name: &str, e: ureq::Error) -> Transfer {
    let msg = format!("download {name}: {e}");
    match e {
        ureq::Error::StatusCode(code) => status_failure(name, code),
        ureq::Error::RequireHttpsOnly(_)
        | ureq::Error::BadUri(_)
        | ureq::Error::Http(_)
        | ureq::Error::InvalidProxyUrl => Transfer::Other(DownloadError::Failed(msg)),
        _ => Transfer::Network(msg),
    }
}

/// Download `target` into a part file under `part_dir` (adopting and
/// resuming an unowned part of the same file when the server allows) and
/// verify it against the index's sha256.
///
/// On success the verified part is returned, still locked: the caller
/// installs from [`PartFile::path`], then [`PartFile::remove`]s it (or
/// [`PartFile::keep`]s it when the install was abandoned, so the next
/// download finds it whole). On failure the part is already dealt with: a
/// network failure or an abandon keeps it, anything else removes it.
pub fn download(
    agent: &ureq::Agent,
    target: &Target,
    part_dir: &Path,
    nonce: &str,
    ctl: &dyn Control,
) -> Result<PartFile, DownloadError> {
    std::fs::create_dir_all(part_dir).map_err(|e| io_failure("mkdir", part_dir, e))?;
    let mut part = PartFile::acquire(part_dir, target, nonce)?;

    // Refuse before a byte moves when the index says how big the file is.
    if let Some(total) = target.bytes {
        if let Err(e) = check_space(ctl, part_dir, &target.name, total, part.len()) {
            // Kept bytes stay for when there is room; an empty part goes.
            if part.is_empty() {
                part.remove();
            } else {
                part.keep();
            }
            return Err(e);
        }
    }

    let hasher = match transfer(agent, target, ctl, part_dir, &mut part) {
        Ok(h) => h,
        // A network failure or an abandon keeps the `.part` for a resume;
        // a cancel, a full disk or a definite refusal removes it.
        Err(Transfer::Network(e)) => {
            part.keep();
            return Err(DownloadError::Failed(e));
        }
        Err(Transfer::Other(DownloadError::Abandoned)) => {
            part.keep();
            return Err(DownloadError::Abandoned);
        }
        Err(Transfer::Other(f)) => {
            part.remove();
            return Err(f);
        }
    };

    if let Some(expected) = &target.sha256 {
        ctl.verifying();
        let actual = hex(&hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            part.remove();
            return Err(DownloadError::ChecksumMismatch {
                name: target.name.clone(),
                actual,
                expected: expected.clone(),
            });
        }
    }
    Ok(part)
}

/// Refuse when the disk under `root` cannot take `total` bytes (times
/// [`DOWNLOAD_SPACE_FACTOR`]) beyond the `have` already downloaded.
fn check_space(
    ctl: &dyn Control,
    root: &Path,
    name: &str,
    total: u64,
    have: u64,
) -> Result<(), DownloadError> {
    let needed = ((total as f64 * DOWNLOAD_SPACE_FACTOR) as u64).saturating_sub(have);
    match ctl.free_space(root) {
        Some(available) if available < needed => Err(DownloadError::Failed(format!(
            "not enough free space to download \"{name}\": needs {}, {} available ({} short)",
            format_bytes(needed),
            format_bytes(available),
            format_bytes(needed - available),
        ))),
        _ => Ok(()),
    }
}

/// Stream the file into `part`, resuming from what is there when the
/// server answers a validated Range request with a matching 206. Returns
/// the sha256 of the whole file, computed as it streamed.
fn transfer(
    agent: &ureq::Agent,
    target: &Target,
    ctl: &dyn Control,
    root: &Path,
    part: &mut PartFile,
) -> Result<Sha256, Transfer> {
    let url = part.meta.url.clone();
    let name = target.name.as_str();
    let get = || {
        agent
            .get(&url)
            // A zip is not compressed again in transit, and a Range must
            // count bytes of the file itself.
            .header("Accept-Encoding", "identity")
    };
    let mut have = part.len();
    if have > 0 && part.meta.if_range().is_none() {
        // Nothing to tell the server which version these bytes are of.
        part.truncate()?;
        have = 0;
    }

    let mut resp = if have > 0 {
        let validator = part.meta.if_range().unwrap_or_default();
        get()
            .header("Range", format!("bytes={have}-"))
            .header("If-Range", validator)
            .call()
            .map_err(|e| request_failure(name, e))?
    } else {
        get().call().map_err(|e| request_failure(name, e))?
    };

    let mut start = 0;
    if have > 0 {
        let code = resp.status().as_u16();
        let (range_start, range_total) = content_range(&resp);
        let total_ok =
            |t: u64| target.bytes.is_none_or(|b| b == t) && part.meta.total.is_none_or(|m| m == t);
        match code {
            // The part is already the whole file the server has.
            416 if range_total == Some(have) && target.bytes.is_none_or(|b| b == have) => {
                let hasher = seed_hash(part, have, ctl)?;
                ctl.progress(Progress {
                    downloaded_bytes: have,
                    total_bytes: have,
                    resumed_from: have,
                    ..Progress::default()
                });
                return Ok(hasher);
            }
            206 if range_start == Some(have) && range_total.is_none_or(total_ok) => {
                start = have;
            }
            // 200 (the file changed: If-Range failed), 416 for another
            // size, or a 206 that does not continue these bytes: over.
            200 => {}
            c if c >= 400 && c != 416 => return Err(status_failure(name, c)),
            _ => {
                drop(resp);
                part.truncate()?;
                resp = get().call().map_err(|e| request_failure(name, e))?;
            }
        }
    }
    let code = resp.status().as_u16();
    if start == 0 && code != 200 {
        return Err(status_failure(name, code));
    }

    let content_length: Option<u64> = header(&resp, "Content-Length").and_then(|s| s.parse().ok());
    let total = content_length
        .map(|n| n + start)
        .or(target.bytes)
        .unwrap_or(0);
    if target.bytes.is_none() && total > 0 {
        check_space(ctl, root, name, total, start)?;
    }

    let mut hasher = if start > 0 {
        seed_hash(part, start, ctl)?
    } else {
        part.truncate()?;
        Sha256::new()
    };
    // What the next resume validates against.
    if start == 0 {
        part.meta.etag = header(&resp, "ETag");
        part.meta.last_modified = header(&resp, "Last-Modified");
    }
    part.meta.total = (total > 0).then_some(total);
    part.write_meta()?;

    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut downloaded = start;
    let mut rate = Rate::new(start);
    let file = part.file();
    loop {
        if let Some(stop) = ctl.stop() {
            return Err(stop.into());
        }
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                // The abort can surface as a read error of a dropped
                // connection; report it as what it is.
                if let Some(stop) = ctl.stop() {
                    return Err(stop.into());
                }
                return Err(Transfer::Network(format!("read body: {e}")));
            }
        };
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| Transfer::Other(DownloadError::Failed(format!("write part file: {e}"))))?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;
        let (bytes_per_sec, eta_secs) = rate.update(downloaded, total);
        ctl.progress(Progress {
            downloaded_bytes: downloaded,
            total_bytes: total,
            bytes_per_sec,
            eta_secs,
            resumed_from: start,
        });
    }
    file.flush()
        .map_err(|e| Transfer::Other(DownloadError::Failed(format!("write part file: {e}"))))?;
    if total > 0 && downloaded < total {
        return Err(Transfer::Network(format!(
            "download {name}: connection closed after {} of {}",
            format_bytes(downloaded),
            format_bytes(total)
        )));
    }
    Ok(hasher)
}

/// The sha256 state after the first `len` bytes of the part (a resume
/// continues it), checking the stop conditions every chunk. Leaves the
/// file positioned at `len`, truncated to it.
fn seed_hash(part: &mut PartFile, len: u64, ctl: &dyn Control) -> Result<Sha256, Transfer> {
    let path = part.path().to_path_buf();
    let err = |what: &str, e: std::io::Error| Transfer::Other(io_failure(what, &path, e));
    let file = part.file();
    file.set_len(len).map_err(|e| err("truncate", e))?;
    file.seek(SeekFrom::Start(0)).map_err(|e| err("seek", e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut left = len;
    while left > 0 {
        if let Some(stop) = ctl.stop() {
            return Err(stop.into());
        }
        let want = buf.len().min(left as usize);
        let n = file.read(&mut buf[..want]).map_err(|e| err("read", e))?;
        if n == 0 {
            return Err(Transfer::Other(DownloadError::Failed(format!(
                "read {}: shorter than expected",
                path.display()
            ))));
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    file.seek(SeekFrom::Start(len))
        .map_err(|e| err("seek", e))?;
    Ok(hasher)
}

/// A smoothed transfer rate and the ETA it gives.
struct Rate {
    window_start: Instant,
    window_bytes: u64,
    bytes_per_sec: f64,
}

impl Rate {
    const WINDOW: Duration = Duration::from_millis(250);

    fn new(start_bytes: u64) -> Self {
        Self {
            window_start: Instant::now(),
            window_bytes: start_bytes,
            bytes_per_sec: 0.0,
        }
    }

    fn update(&mut self, downloaded: u64, total: u64) -> (f64, Option<f64>) {
        let elapsed = self.window_start.elapsed();
        if elapsed >= Self::WINDOW {
            let now = (downloaded - self.window_bytes) as f64 / elapsed.as_secs_f64();
            self.bytes_per_sec = if self.bytes_per_sec == 0.0 {
                now
            } else {
                0.7 * self.bytes_per_sec + 0.3 * now
            };
            self.window_start = Instant::now();
            self.window_bytes = downloaded;
        }
        let eta = (self.bytes_per_sec > 0.0 && total > downloaded)
            .then(|| (total - downloaded) as f64 / self.bytes_per_sec);
        (self.bytes_per_sec, eta)
    }
}
