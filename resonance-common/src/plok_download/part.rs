//! Part files: where a download streams, and how an interrupted one is
//! resumed.
//!
//! A transfer that fails on the network (a dropped connection, a stall, a
//! 5xx/408/429 answer) or is abandoned keeps its `.part`, named after the
//! index entry's *file* (plus a hash of its size and sha256) and a
//! per-worker random nonce: `.<file>-<hash>.<nonce>.zip.part`, with a
//! `.meta` beside it holding the URL, the server's validators (ETag /
//! Last-Modified) and the total. The part is held under an exclusive
//! advisory lock (`File::try_lock`) while it is written; an unlocked part
//! is nobody's, and the next download of that file — in this process or
//! another — adopts it (renaming it to its own nonce) and asks for
//! `Range: bytes=<len>-` with `If-Range`. A part without a validator is
//! never resumed.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::transfer::Target;
use super::{hex, DownloadError};
use crate::library_marks;

/// Suffix of the file a download streams into.
const PART_SUFFIX: &str = ".zip.part";
/// Suffix of a part's validator record, beside it.
const META_SUFFIX: &str = ".meta";
/// Suffix of the directory the pre-library drums worker extracted into
/// before moving it into place. Nothing writes these any more; leftovers
/// are swept.
const STAGING_SUFFIX: &str = ".extracting";

/// An unlocked part (nobody is writing it) is removed after this long
/// without a write: a resume an hour later is not worth keeping gigabytes
/// around for.
const STALE_DOWNLOAD_AGE: Duration = Duration::from_secs(60 * 60);

/// A `.meta` with no part beside it, or a part with no `.meta`, is removed
/// after this long (a part is created and locked right before its meta).
const ORPHAN_META_AGE: Duration = Duration::from_secs(60);

/// A per-worker random tag: the hasher keys of `RandomState` are random
/// per process, the counter and the clock make it unique within one.
pub fn new_nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    format!("{:016x}", h.finish())
}

/// What a `.part` is a download of, and how to tell the server which
/// version of the file its bytes are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(super) struct PartMeta {
    pub url: String,
    file: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// The whole file's size, once a server (or the index) said.
    #[serde(default)]
    pub total: Option<u64>,
    /// The worker writing it (informational: liveness is the lock).
    #[serde(default)]
    nonce: String,
}

impl PartMeta {
    fn new(target: &Target, nonce: &str) -> Self {
        Self {
            url: target.url.clone(),
            file: target.file.clone(),
            sha256: target.sha256_lower(),
            etag: None,
            last_modified: None,
            total: target.bytes,
            nonce: nonce.to_string(),
        }
    }

    /// The `If-Range` validator: a strong ETag, else Last-Modified.
    pub fn if_range(&self) -> Option<String> {
        self.etag
            .clone()
            .filter(|e| !e.starts_with("W/"))
            .or_else(|| self.last_modified.clone())
    }

    /// Whether a kept part with this record can continue a download of
    /// `target`.
    fn continues(&self, target: &Target) -> bool {
        let sha = target.sha256_lower();
        self.url == target.url
            && self.file == target.file
            && (sha.is_none() || self.sha256 == sha)
            && target
                .bytes
                .is_none_or(|b| self.total.is_none_or(|t| t == b))
            && self.if_range().is_some()
    }
}

/// `.<file>-<hash>`: the index entry's file name (not the display name —
/// "Kit (A)" and "Kit A" sanitize alike) and a hash of the file, its size
/// and its sha256, so a new version of the file never continues an old
/// part.
fn part_stem(target: &Target) -> String {
    let file = target.file.trim();
    let base = file.rsplit('/').next().unwrap_or(file);
    let base = base.strip_suffix(".zip").unwrap_or(base);
    let mut h = Sha256::new();
    h.update(file.as_bytes());
    h.update([0]);
    h.update(target.bytes.unwrap_or(0).to_le_bytes());
    h.update([0]);
    h.update(target.sha256_lower().unwrap_or_default().as_bytes());
    let digest = hex(&h.finalize());
    format!(".{}-{}", sanitize(base), &digest[..12])
}

/// The `.part` a download of `target` by the worker with `nonce` streams
/// into, under `dir`.
pub fn part_path(dir: &Path, target: &Target, nonce: &str) -> PathBuf {
    dir.join(format!("{}.{nonce}{PART_SUFFIX}", part_stem(target)))
}

fn meta_path_of(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_owned();
    s.push(META_SUFFIX);
    PathBuf::from(s)
}

fn read_meta(path: &Path) -> Option<PartMeta> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Whether `file` is now locked by us.
enum Lock {
    Held,
    Busy,
}

fn try_lock(file: &File) -> std::io::Result<Lock> {
    match file.try_lock() {
        Ok(()) => Ok(Lock::Held),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Lock::Busy),
        // No locks on this filesystem (NFS without lockd, some FUSE): go
        // on unlocked, as the library and the marks store do.
        Err(std::fs::TryLockError::Error(e)) if library_marks::lock_error_is_unsupported(&e) => {
            Ok(Lock::Held)
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

pub(super) fn io_failure(what: &str, path: &Path, e: std::io::Error) -> DownloadError {
    DownloadError::Failed(format!("{what} {}: {e}", path.display()))
}

/// The `.part` a download writes, open and locked for as long as it lives.
/// Dropping it without [`keep`](Self::keep) or [`remove`](Self::remove)
/// keeps it (unlocked).
pub struct PartFile {
    path: PathBuf,
    meta_path: PathBuf,
    file: Option<File>,
    pub(super) meta: PartMeta,
}

impl PartFile {
    /// Adopt an unowned part of the same file (renamed to this worker's
    /// name), or start a new one. Parts of the file that are locked belong
    /// to a live download and are left alone; unlocked ones that cannot
    /// continue this download are removed.
    pub(super) fn acquire(
        root: &Path,
        target: &Target,
        nonce: &str,
    ) -> Result<Self, DownloadError> {
        let ours = part_path(root, target, nonce);
        let prefix = format!("{}.", part_stem(target));
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(root)
            .map(|d| {
                d.flatten()
                    .filter(|e| {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        n.starts_with(&prefix) && n.ends_with(PART_SUFFIX)
                    })
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default();
        // Our own first: it is the likeliest to continue.
        candidates.sort_by_key(|p| *p != ours);

        let ours_meta = meta_path_of(&ours);
        let mut adopted = None;
        for path in candidates {
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            if !matches!(try_lock(&file), Ok(Lock::Held)) {
                continue;
            }
            let meta_path = meta_path_of(&path);
            let meta = read_meta(&meta_path).filter(|m| m.continues(target));
            match meta {
                Some(meta) if adopted.is_none() => {
                    if path != ours {
                        if std::fs::rename(&path, &ours).is_err() {
                            // Taken from under us; leave it.
                            continue;
                        }
                        let _ = std::fs::rename(&meta_path, &ours_meta);
                    }
                    adopted = Some((file, meta));
                }
                _ => {
                    remove_logged(&path);
                    remove_logged(&meta_path);
                }
            }
        }

        let (file, mut meta) = match adopted {
            Some(found) => found,
            None => {
                // Open without truncating and empty the file only once the
                // lock is ours: a part another download holds (the same
                // nonce, or a racing adopt) is never cut under it. The
                // record is written after the lock too (below), so it never
                // describes a part somebody else is writing. A sweep leaves
                // a fresh record-less part alone ([`ORPHAN_META_AGE`]).
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&ours)
                    .map_err(|e| io_failure("create", &ours, e))?;
                match try_lock(&file) {
                    Ok(Lock::Held) => {}
                    Ok(Lock::Busy) => {
                        return Err(DownloadError::Failed(format!(
                            "{} is locked by another download",
                            ours.display()
                        )))
                    }
                    Err(e) => return Err(io_failure("lock", &ours, e)),
                }
                file.set_len(0)
                    .map_err(|e| io_failure("truncate", &ours, e))?;
                (file, PartMeta::new(target, nonce))
            }
        };
        meta.nonce = nonce.to_string();
        let part = Self {
            path: ours,
            meta_path: ours_meta,
            file: Some(file),
            meta,
        };
        part.write_meta()?;
        Ok(part)
    }

    /// The part file's path (the downloaded file, once verified).
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn file(&mut self) -> &mut File {
        self.file
            .as_mut()
            .expect("the part file is open until dropped")
    }

    /// Bytes in the part.
    pub fn len(&self) -> u64 {
        self.file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map(|m| m.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Start the file over (the server's copy is another, or the part had
    /// no validator).
    pub(super) fn truncate(&mut self) -> Result<(), DownloadError> {
        let path = self.path.clone();
        let file = self.file();
        file.set_len(0)
            .map_err(|e| io_failure("truncate", &path, e))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| io_failure("seek", &path, e))?;
        self.meta.etag = None;
        self.meta.last_modified = None;
        Ok(())
    }

    pub(super) fn write_meta(&self) -> Result<(), DownloadError> {
        write_meta_file(&self.meta_path, &self.meta)
    }

    /// Keep the part (and its record) for a resume; the lock goes.
    pub fn keep(mut self) {
        if let Some(f) = self.file.take() {
            let _ = f.sync_data();
        }
    }

    /// Delete the part and its record.
    pub fn remove(mut self) {
        remove_logged(&self.path);
        remove_logged(&self.meta_path);
        self.file.take();
    }
}

fn write_meta_file(path: &Path, meta: &PartMeta) -> Result<(), DownloadError> {
    let bytes =
        serde_json::to_vec(meta).map_err(|e| DownloadError::Failed(format!("part record: {e}")))?;
    crate::atomic_file::atomic_write(path, &bytes)
        .map_err(|e| DownloadError::Failed(format!("write {}: {e}", path.display())))
}

pub(super) fn remove_logged(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("remove download leftover {}: {e}", path.display()),
    }
}

fn age_of(meta: &std::fs::Metadata) -> Option<Duration> {
    meta.modified().ok().and_then(|t| t.elapsed().ok())
}

/// Remove download leftovers in `dir` no running download can own:
///
/// - a `.part` nobody holds the lock of, when it has had no record for a
///   minute (it cannot be resumed) or has not been written for an hour —
///   never one that is locked (a live download, in any process);
/// - a record whose part is gone;
/// - the pre-library drums worker's `.extracting` directories.
pub fn sweep_stale_downloads(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if name.ends_with(STAGING_SUFFIX) {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!("remove download leftover {}: {e}", path.display());
            }
        } else if name.ends_with(PART_SUFFIX) {
            let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            if !matches!(try_lock(&file), Ok(Lock::Held)) {
                continue;
            }
            let meta_path = meta_path_of(&path);
            let age = file.metadata().ok().and_then(|m| age_of(&m));
            let stale = age.is_some_and(|age| age > STALE_DOWNLOAD_AGE);
            // A part is created (and locked) a moment before its record is
            // written: only an older one without a record is abandoned.
            let orphan = !meta_path.exists() && age.is_some_and(|age| age > ORPHAN_META_AGE);
            if stale || orphan {
                remove_logged(&path);
                remove_logged(&meta_path);
            }
        } else if let Some(part) = name.strip_suffix(META_SUFFIX) {
            if part.ends_with(PART_SUFFIX) && !dir.join(part).exists() {
                let old = entry
                    .metadata()
                    .ok()
                    .and_then(|m| age_of(&m))
                    .is_some_and(|age| age > ORPHAN_META_AGE);
                if old {
                    remove_logged(&path);
                }
            }
        }
    }
}

/// Conservative filename sanitizer: keep ASCII alphanumerics, `-`, `_`, `.`;
/// replace whitespace with `_`, drop everything else.
pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("kit");
    }
    out
}
