//! The file work behind Import and (later) the plok.org download: walking
//! and measuring a kit, the free-space check, and copying or extracting
//! into `.staging/` in chunks with progress and a cancel flag
//! (drums-plugin-rework.md §4.1, §6.5, D2).
//!
//! None of this holds the library: a caller copies on a background job and
//! then rescans.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::LibraryError;

/// Copy/extract chunk: the cancel flag is checked and progress reported
/// once per chunk.
const CHUNK: usize = 1024 * 1024;

/// How far an import or extraction has got.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportProgress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
}

/// A free-space probe: bytes available at a path, `None` when unknown.
type FreeSpaceFn<'a> = Box<dyn Fn(&Path) -> Option<u64> + 'a>;

/// The knobs of one import: a cancel flag, a progress callback and the
/// free-space probe (injectable so tests can simulate a full disk). All
/// optional; [`ImportJob::default`] has none and probes the real disk.
#[derive(Default)]
pub struct ImportJob<'a> {
    cancel: Option<&'a AtomicBool>,
    progress: Option<Box<dyn FnMut(ImportProgress) + 'a>>,
    free_space: Option<FreeSpaceFn<'a>>,
}

impl<'a> ImportJob<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Abandon the import at the next chunk once `flag` is set; nothing is
    /// left behind.
    pub fn cancel(mut self, flag: &'a AtomicBool) -> Self {
        self.cancel = Some(flag);
        self
    }

    /// Called after every chunk and every file.
    pub fn progress(mut self, f: impl FnMut(ImportProgress) + 'a) -> Self {
        self.progress = Some(Box::new(f));
        self
    }

    /// Replace the [`free_space()`](super::free_space) probe (bytes
    /// available at a path; `None` = unknown, which skips the check).
    pub fn free_space(mut self, f: impl Fn(&Path) -> Option<u64> + 'a) -> Self {
        self.free_space = Some(Box::new(f));
        self
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }

    fn check_cancel(&self) -> Result<(), LibraryError> {
        if self.is_cancelled() {
            Err(LibraryError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn report(&mut self, p: ImportProgress) {
        if let Some(f) = self.progress.as_mut() {
            f(p);
        }
    }

    /// Refuse unless the disk under `at` has `needed` bytes free.
    pub(super) fn check_space(&self, at: &Path, needed: u64) -> Result<(), LibraryError> {
        let available = match &self.free_space {
            Some(f) => f(at),
            None => free_space(at),
        };
        match available {
            Some(available) if available < needed => {
                Err(LibraryError::InsufficientSpace { needed, available })
            }
            _ => Ok(()),
        }
    }
}

/// Bytes available to this user on the filesystem holding `path`, or
/// `None` when it cannot be determined (then no check is made).
#[cfg(unix)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path and `st` a writable statvfs.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Bytes available at `path`: unknown on this platform.
#[cfg(not(unix))]
pub fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// The regular files under `dir`, recursively, as (relative path, length),
/// sorted. Symlinks are not followed (nor listed).
pub(super) fn walk_files(dir: &Path) -> std::io::Result<Vec<(PathBuf, u64)>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> std::io::Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let t = e.file_type()?;
            let p = e.path();
            if t.is_dir() {
                walk(base, &p, out)?;
            } else if t.is_file() {
                let len = e.metadata()?.len();
                out.push((p.strip_prefix(base).unwrap_or(&p).to_path_buf(), len));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// Total bytes of the regular files under `dir` (symlinks not followed).
/// Drummica is ~2,800 files, so callers run this on a background thread
/// and store the result with [`super::Library::record_size`].
pub fn measure_size(dir: &Path) -> std::io::Result<u64> {
    Ok(walk_files(dir)?.iter().map(|(_, n)| n).sum())
}

fn io_err<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl FnOnce(std::io::Error) -> LibraryError + 'a {
    move |source| LibraryError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// Stream `src` into a new file at `dest` in chunks.
fn copy_stream(
    src: &mut dyn Read,
    dest: &Path,
    job: &mut ImportJob<'_>,
    progress: &mut ImportProgress,
) -> Result<(), LibraryError> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(io_err("mkdir", dir))?;
    }
    let mut out = File::create(dest).map_err(io_err("create", dest))?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        job.check_cancel()?;
        let n = src.read(&mut buf).map_err(io_err("read", dest))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io_err("write", dest))?;
        progress.bytes_done += n as u64;
        job.report(*progress);
    }
    progress.files_done += 1;
    job.report(*progress);
    Ok(())
}

/// Copy the tree `files` (from [`walk_files`] over `src`) to `dest`.
pub(super) fn copy_tree(
    src: &Path,
    files: &[(PathBuf, u64)],
    dest: &Path,
    job: &mut ImportJob<'_>,
) -> Result<u64, LibraryError> {
    std::fs::create_dir_all(dest).map_err(io_err("mkdir", dest))?;
    let mut progress = ImportProgress {
        bytes_total: files.iter().map(|(_, n)| n).sum(),
        files_total: files.len() as u64,
        ..ImportProgress::default()
    };
    job.report(progress);
    for (rel, _) in files {
        let from = src.join(rel);
        let mut f = File::open(&from).map_err(io_err("open", &from))?;
        copy_stream(&mut f, &dest.join(rel), job, &mut progress)?;
    }
    Ok(progress.bytes_done)
}

/// The files a zip would extract: (archive index, enclosed path, size).
/// Entries with an unsafe path (absolute, `..`) are skipped.
pub(super) fn zip_plan(
    archive: &mut zip::ZipArchive<File>,
) -> Result<Vec<(usize, PathBuf, u64)>, LibraryError> {
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let e = archive
            .by_index(i)
            .map_err(|e| LibraryError::Zip(format!("entry {i}: {e}")))?;
        if e.is_dir() {
            continue;
        }
        if let Some(path) = e.enclosed_name() {
            out.push((i, path, e.size()));
        }
    }
    Ok(out)
}

/// Extract `plan` (from [`zip_plan`]) under `dest`.
pub(super) fn extract(
    archive: &mut zip::ZipArchive<File>,
    plan: &[(usize, PathBuf, u64)],
    dest: &Path,
    job: &mut ImportJob<'_>,
) -> Result<u64, LibraryError> {
    std::fs::create_dir_all(dest).map_err(io_err("mkdir", dest))?;
    let mut progress = ImportProgress {
        bytes_total: plan.iter().map(|(_, _, n)| n).sum(),
        files_total: plan.len() as u64,
        ..ImportProgress::default()
    };
    job.report(progress);
    for (i, rel, _) in plan {
        let mut e = archive
            .by_index(*i)
            .map_err(|e| LibraryError::Zip(format!("entry {i}: {e}")))?;
        copy_stream(&mut e, &dest.join(rel), job, &mut progress)?;
    }
    Ok(progress.bytes_done)
}
