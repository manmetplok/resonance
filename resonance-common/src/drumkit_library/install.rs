//! The file work behind Import and the plok.org download: walking and
//! measuring a kit, the free-space check, and copying or extracting into
//! `.staging/` in chunks with progress and a cancel flag
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

/// How deep a folder walk goes before it gives up (a symlink loop the
/// visited set did not catch, or an absurd tree).
const MAX_WALK_DEPTH: usize = 32;

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

/// One folder walk: the files found, and the canonical directories on the
/// current path (a symlinked directory that resolves to one of them is a
/// loop).
struct Walk {
    base: PathBuf,
    canon_base: PathBuf,
    /// Refuse (rather than skip) a symlinked directory that escapes the
    /// walked folder or loops, or a tree deeper than [`MAX_WALK_DEPTH`].
    strict: bool,
    stack: Vec<PathBuf>,
    out: Vec<(PathBuf, u64)>,
}

impl Walk {
    fn refuse(&self, path: &Path, reason: &'static str) -> Result<(), LibraryError> {
        if self.strict {
            Err(LibraryError::UnsafeLink {
                path: path.to_path_buf(),
                reason,
            })
        } else {
            Ok(())
        }
    }

    fn push_file(&mut self, p: &Path, len: u64) {
        let rel = p.strip_prefix(&self.base).unwrap_or(p).to_path_buf();
        self.out.push((rel, len));
    }

    fn dir(&mut self, dir: &Path, canon: PathBuf) -> Result<(), LibraryError> {
        if self.stack.len() > MAX_WALK_DEPTH {
            return self.refuse(dir, "is nested too deeply");
        }
        self.stack.push(canon);
        let read = |e: std::io::Error| LibraryError::Io {
            op: "read",
            path: dir.to_path_buf(),
            source: e,
        };
        for e in std::fs::read_dir(dir).map_err(read)? {
            let e = e.map_err(read)?;
            let t = e.file_type().map_err(read)?;
            let p = e.path();
            if t.is_symlink() {
                // Follow it: a symlinked sample or manifest is copied as
                // the file it points at.
                let Ok(meta) = std::fs::metadata(&p) else {
                    // Dangling: missing at the source too.
                    continue;
                };
                if meta.is_file() {
                    self.push_file(&p, meta.len());
                } else if meta.is_dir() {
                    let Ok(canon) = std::fs::canonicalize(&p) else {
                        self.refuse(&p, "cannot be resolved")?;
                        continue;
                    };
                    if !canon.starts_with(&self.canon_base) {
                        self.refuse(&p, "is a symlinked folder that points outside the kit")?;
                    } else if self.stack.contains(&canon) {
                        self.refuse(&p, "is a symlinked folder that loops back on itself")?;
                    } else {
                        self.dir(&p, canon)?;
                    }
                }
                // A fifo or device behind a link is not kit content.
            } else if t.is_dir() {
                let canon = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                self.dir(&p, canon)?;
            } else if t.is_file() {
                let len = e.metadata().map_err(read)?.len();
                self.push_file(&p, len);
            }
        }
        self.stack.pop();
        Ok(())
    }
}

fn walk(dir: &Path, strict: bool) -> Result<Vec<(PathBuf, u64)>, LibraryError> {
    let canon_base = std::fs::canonicalize(dir).map_err(|source| LibraryError::Io {
        op: "resolve",
        path: dir.to_path_buf(),
        source,
    })?;
    let mut w = Walk {
        base: dir.to_path_buf(),
        canon_base: canon_base.clone(),
        strict,
        stack: Vec::new(),
        out: Vec::new(),
    };
    w.dir(dir, canon_base)?;
    w.out.sort();
    Ok(w.out)
}

/// The files under `dir`, recursively, as (relative path, length), sorted.
/// A symlinked file is listed with its target's length (and copied as its
/// target); a dangling one is skipped. A symlinked folder is followed when
/// it resolves inside `dir`; one that points outside it, or back at a
/// folder it is inside of, refuses the walk ([`LibraryError::UnsafeLink`]).
pub(super) fn walk_files(dir: &Path) -> Result<Vec<(PathBuf, u64)>, LibraryError> {
    walk(dir, true)
}

/// Total bytes of the files under `dir`, counted as [`walk_files`] would
/// copy them, except that a symlinked folder that escapes or loops is
/// skipped rather than refused. Drummica is ~2,800 files, so callers run
/// this on a background thread and store the result with
/// [`super::Library::record_size`].
pub fn measure_size(dir: &Path) -> std::io::Result<u64> {
    match walk(dir, false) {
        Ok(files) => Ok(files.iter().map(|(_, n)| n).sum()),
        Err(LibraryError::Io { source, .. }) => Err(source),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
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

/// Stream `src` into a new file at `dest` in chunks of `buf`, and fsync it
/// before it is closed (a promoted kit must not turn out empty after a
/// crash). Returns the bytes written.
fn copy_stream(
    src: &mut dyn Read,
    dest: &Path,
    buf: &mut [u8],
    job: &mut ImportJob<'_>,
    progress: &mut ImportProgress,
) -> Result<u64, LibraryError> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(io_err("mkdir", dir))?;
    }
    let mut out = File::create(dest).map_err(io_err("create", dest))?;
    let mut written = 0u64;
    loop {
        job.check_cancel()?;
        let n = src.read(buf).map_err(io_err("read", dest))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io_err("write", dest))?;
        written += n as u64;
        progress.bytes_done += n as u64;
        job.report(*progress);
    }
    out.sync_all().map_err(io_err("fsync", dest))?;
    progress.files_done += 1;
    job.report(*progress);
    Ok(written)
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
    let mut buf = vec![0u8; CHUNK];
    for (rel, _) in files {
        let from = src.join(rel);
        let mut f = File::open(&from).map_err(io_err("open", &from))?;
        copy_stream(&mut f, &dest.join(rel), &mut buf, job, &mut progress)?;
    }
    Ok(progress.bytes_done)
}

#[cfg(feature = "drumkit-zip")]
pub(super) use zip_impl::{extract, read_entry, zip_plan, ZipPlan};

#[cfg(feature = "drumkit-zip")]
mod zip_impl {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::Read;
    use std::path::{Component, Path, PathBuf};

    use super::{copy_stream, io_err, ImportJob, ImportProgress, CHUNK};
    use crate::drumkit_library::{LibraryError, MANIFEST_FILE};

    /// A manifest bigger than this is not read into memory.
    const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

    /// What a zip would extract, and where its kit's manifest is.
    pub(in crate::drumkit_library) struct ZipPlan {
        /// (archive index, path under the kit's top directory, declared
        /// size): every file but macOS litter (`__MACOSX/`, dotfiles) and
        /// unsafe paths (absolute, `..`), with a lone wrapper directory
        /// around a depth-1 kit stripped.
        pub files: Vec<(usize, PathBuf, u64)>,
        /// The archive index of the kit's `drum_samples.json`.
        pub manifest: usize,
        /// Its declared size.
        pub manifest_size: u64,
    }

    impl ZipPlan {
        pub fn total(&self) -> u64 {
            self.files.iter().map(|(_, _, n)| n).sum()
        }
    }

    fn zip_err(i: usize) -> impl FnOnce(zip::result::ZipError) -> LibraryError {
        move |e| LibraryError::Zip(format!("entry {i}: {e}"))
    }

    /// macOS resource forks (`__MACOSX/…`) and dotfiles (`.DS_Store`,
    /// `._x`), at any depth: never kit content, and they would otherwise
    /// count as top-level entries.
    fn is_litter(path: &Path) -> bool {
        path.components().any(|c| match c {
            Component::Normal(s) => s == "__MACOSX" || s.to_string_lossy().starts_with('.'),
            _ => false,
        })
    }

    /// The manifest among `paths` (relative to the kit's top directory):
    /// at depth 0, else the one at depth 1. Several at depth 1 and none at
    /// depth 0 is several kits.
    fn find_manifest_in(
        paths: &[(usize, PathBuf, u64)],
        zip: &Path,
    ) -> Result<Option<usize>, LibraryError> {
        let mut depth1: Vec<(&OsStr, usize)> = Vec::new();
        for (pos, (_, p, _)) in paths.iter().enumerate() {
            if p.file_name() != Some(OsStr::new(MANIFEST_FILE)) {
                continue;
            }
            let mut comps = p.components();
            match (comps.next(), comps.next(), comps.next()) {
                (Some(_), None, _) => return Ok(Some(pos)),
                (Some(Component::Normal(dir)), Some(_), None) => depth1.push((dir, pos)),
                _ => {}
            }
        }
        match depth1.len() {
            0 => Ok(None),
            1 => Ok(Some(depth1[0].1)),
            count => Err(LibraryError::MultipleKits {
                path: zip.to_path_buf(),
                count,
            }),
        }
    }

    /// Plan the extraction of the kit in `archive` (named `zip` in
    /// errors). The manifest may sit at depth 0 or 1, or at depth 1 under
    /// a single wrapper directory (`Kit/kit/drum_samples.json`), which is
    /// stripped.
    pub fn zip_plan(
        archive: &mut zip::ZipArchive<File>,
        zip: &Path,
    ) -> Result<ZipPlan, LibraryError> {
        let mut files = Vec::new();
        for i in 0..archive.len() {
            let e = archive.by_index_raw(i).map_err(zip_err(i))?;
            if e.is_dir() {
                continue;
            }
            let Some(path) = e.enclosed_name() else {
                continue;
            };
            if is_litter(&path) {
                continue;
            }
            files.push((i, path, e.size()));
        }
        let mut found = find_manifest_in(&files, zip)?;
        if found.is_none() {
            // One wrapper directory around everything: look inside it.
            let first = |p: &PathBuf| p.components().next().map(|c| c.as_os_str().to_owned());
            let wrapper = files.first().and_then(|(_, p, _)| first(p));
            let wrapped = wrapper.as_ref().is_some_and(|w| {
                files
                    .iter()
                    .all(|(_, p, _)| p.components().count() > 1 && first(p).as_ref() == Some(w))
            });
            if let (Some(w), true) = (wrapper, wrapped) {
                for (_, p, _) in files.iter_mut() {
                    *p = p
                        .strip_prefix(&w)
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                }
                found = find_manifest_in(&files, zip)?;
            }
        }
        let pos = found.ok_or_else(|| LibraryError::NotAKit {
            path: zip.to_path_buf(),
            reason: "no drum_samples.json in the archive".into(),
        })?;
        let (manifest, _, manifest_size) = files[pos];
        Ok(ZipPlan {
            files,
            manifest,
            manifest_size,
        })
    }

    /// The bytes of archive entry `i`, declared `declared` bytes long;
    /// refused when it is bigger than [`MAX_MANIFEST_BYTES`] or inflates
    /// past what it declares.
    pub fn read_entry(
        archive: &mut zip::ZipArchive<File>,
        i: usize,
        declared: u64,
    ) -> Result<Vec<u8>, LibraryError> {
        if declared > MAX_MANIFEST_BYTES {
            return Err(LibraryError::Zip(format!(
                "entry {i}: a {declared}-byte manifest is too large"
            )));
        }
        let e = archive.by_index(i).map_err(zip_err(i))?;
        let mut bytes = Vec::with_capacity(declared as usize);
        e.take(declared + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| LibraryError::Zip(format!("entry {i}: {e}")))?;
        if bytes.len() as u64 > declared {
            return Err(oversize(i));
        }
        Ok(bytes)
    }

    fn oversize(i: usize) -> LibraryError {
        LibraryError::Zip(format!("entry {i} inflates past its declared size"))
    }

    /// Extract `plan` under `dest`. Each entry is read through a cap of
    /// its declared size + 1, and one that yields more is refused, so a
    /// zip bomb cannot outgrow the disk check.
    pub fn extract(
        archive: &mut zip::ZipArchive<File>,
        plan: &ZipPlan,
        dest: &Path,
        job: &mut ImportJob<'_>,
    ) -> Result<u64, LibraryError> {
        std::fs::create_dir_all(dest).map_err(io_err("mkdir", dest))?;
        let mut progress = ImportProgress {
            bytes_total: plan.total(),
            files_total: plan.files.len() as u64,
            ..ImportProgress::default()
        };
        job.report(progress);
        let mut buf = vec![0u8; CHUNK];
        for (i, rel, declared) in &plan.files {
            let e = archive.by_index(*i).map_err(zip_err(*i))?;
            let mut capped = e.take(declared + 1);
            let n = copy_stream(&mut capped, &dest.join(rel), &mut buf, job, &mut progress)?;
            if n > *declared {
                return Err(oversize(*i));
            }
        }
        Ok(progress.bytes_done)
    }
}
