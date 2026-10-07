//! Installing a downloaded zip as a folder: a voicebank, a vocoder — any
//! content that is "a folder with a marker file near its top".
//!
//! The archive is planned first (macOS litter and unsafe paths skipped, a
//! wrapper directory stripped), the disk is checked, the files are
//! extracted into a hidden staging folder beside the destination in
//! chunks with progress and cancel, and the staging folder is renamed into
//! place — replacing an existing install only once the new one is whole.
//! A cancelled or failed install leaves nothing behind.
//!
//! **Where the content's top is.** The marker (`dsconfig.yaml`,
//! `vocoder.yaml`) nearest the top decides:
//! - while it sits deeper than the rule allows, the folders above the
//!   allowed depth are dropped, and with them any file outside that branch
//!   (a pack's readme next to the bank folder);
//! - then, while every file sits under one directory and the marker is not
//!   at the top, that wrapper directory is stripped
//!   (`Bank_v1/dsconfig.yaml` installs as `<dest>/dsconfig.yaml`).

use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::drumkit_library::install::{copy_stream, is_litter, CHUNK};
use crate::drumkit_library::{ImportJob, ImportProgress, LibraryError};

/// What the content looks like.
pub struct PackageRule<'a> {
    /// What it is, for messages ("voicebank", "vocoder").
    pub what: &'a str,
    /// Whether an archive path (relative, `/`-separated components) is the
    /// content's marker file.
    pub is_marker: &'a dyn Fn(&Path) -> bool,
    /// How many folders below the installed folder the marker may sit
    /// (0 = at its top).
    pub max_depth: usize,
}

/// Why a package did not install.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("{} is not a {what}: {reason}", path.display())]
    NotAPackage {
        path: PathBuf,
        what: String,
        reason: String,
    },
    #[error(transparent)]
    Library(#[from] LibraryError),
}

impl PackageError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, PackageError::Library(LibraryError::Cancelled))
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

fn zip_err(i: usize) -> impl FnOnce(zip::result::ZipError) -> LibraryError {
    move |e| LibraryError::Zip(format!("entry {i}: {e}"))
}

/// One archive file to extract: (archive index, path under the content's
/// top, declared size).
type Planned = (usize, PathBuf, u64);

/// The files of `archive` under the content's top (see the module docs).
fn plan(
    archive: &mut zip::ZipArchive<File>,
    zip: &Path,
    rule: &PackageRule<'_>,
) -> Result<Vec<Planned>, PackageError> {
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
    let not_a_package = |reason: String| PackageError::NotAPackage {
        path: zip.to_path_buf(),
        what: rule.what.to_string(),
        reason,
    };
    loop {
        let marker = files
            .iter()
            .filter(|(_, p, _)| (rule.is_marker)(p))
            .map(|(_, p, _)| p)
            .min_by_key(|p| (p.components().count(), p.to_path_buf()))
            .cloned()
            .ok_or_else(|| not_a_package("the archive holds none of its files".into()))?;
        let depth = marker.components().count() - 1;
        let Some(Component::Normal(first)) = marker.components().next() else {
            break;
        };
        let first: OsString = first.to_owned();
        let all_under = files
            .iter()
            .all(|(_, p, _)| p.components().count() > 1 && p.starts_with(&first));
        if depth > rule.max_depth || (depth > 0 && all_under) {
            let before = files.len();
            files.retain(|(_, p, _)| p.components().count() > 1 && p.starts_with(&first));
            if files.len() != before {
                tracing::debug!(
                    "{}: {} file(s) outside {:?} are not part of the {}",
                    zip.display(),
                    before - files.len(),
                    first,
                    rule.what
                );
            }
            for (_, p, _) in files.iter_mut() {
                *p = p
                    .strip_prefix(&first)
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
            }
            continue;
        }
        break;
    }
    if files.is_empty() {
        return Err(not_a_package("the archive is empty".into()));
    }
    Ok(files)
}

/// Install the content of `zip` as the folder `dest` (see the module
/// docs). Returns the bytes extracted. `dest`'s parent is created; an
/// existing `dest` is replaced once the new content is whole.
pub fn install_package(
    zip: &Path,
    dest: &Path,
    rule: &PackageRule<'_>,
    mut job: ImportJob<'_>,
) -> Result<u64, PackageError> {
    let parent = dest
        .parent()
        .ok_or_else(|| LibraryError::Zip(format!("{} has no parent folder", dest.display())))?;
    std::fs::create_dir_all(parent).map_err(io_err("mkdir", parent))?;
    let file = File::open(zip).map_err(io_err("open", zip))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| LibraryError::Zip(format!("{}: {e}", zip.display())))?;
    let files = plan(&mut archive, zip, rule)?;
    let total: u64 = files.iter().map(|(_, _, n)| n).sum();
    job.check_space(parent, total)?;

    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "package".into());
    let staging = parent.join(aside_name(&name, Aside::Staging));
    let result = extract(&mut archive, &files, &staging, &mut job);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.into());
    }
    promote(&staging, dest, parent, &name).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&staging);
    })?;
    Ok(total)
}

/// Extract `files` under `dest`. Each entry is read through a cap of its
/// declared size + 1, and one that yields more is refused, so a zip bomb
/// cannot outgrow the disk check.
fn extract(
    archive: &mut zip::ZipArchive<File>,
    files: &[Planned],
    dest: &Path,
    job: &mut ImportJob<'_>,
) -> Result<(), LibraryError> {
    std::fs::create_dir_all(dest).map_err(io_err("mkdir", dest))?;
    let mut progress = ImportProgress {
        bytes_total: files.iter().map(|(_, _, n)| n).sum(),
        files_total: files.len() as u64,
        ..ImportProgress::default()
    };
    job.report(progress);
    let mut buf = vec![0u8; CHUNK];
    for (i, rel, declared) in files {
        job.check_cancel()?;
        let e = archive.by_index(*i).map_err(zip_err(*i))?;
        let mut capped = e.take(declared + 1);
        let n = copy_stream(&mut capped, &dest.join(rel), &mut buf, job, &mut progress)?;
        if n > *declared {
            return Err(LibraryError::Zip(format!(
                "entry {i} inflates past its declared size"
            )));
        }
    }
    Ok(())
}

/// Move the whole `staging` folder to `dest`, setting an existing `dest`
/// aside first and deleting it only once the new one is in place.
fn promote(staging: &Path, dest: &Path, parent: &Path, name: &str) -> Result<(), LibraryError> {
    let old = parent.join(aside_name(name, Aside::Old));
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(dest, &old).map_err(io_err("move aside", dest))?;
    }
    if let Err(e) = std::fs::rename(staging, dest) {
        if had_old {
            let _ = std::fs::rename(&old, dest);
        }
        return Err(io_err("rename", staging)(e));
    }
    if had_old {
        if let Err(e) = std::fs::remove_dir_all(&old) {
            tracing::warn!("remove replaced {}: {e}", old.display());
        }
    }
    Ok(())
}

/// The hidden folders an install (or a Delete) sets beside the content
/// folder `<name>`: `.<name>.<kind>-<pid>-<nonce>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aside {
    /// What an install extracts into before it is renamed into place.
    Staging,
    /// The previous install, moved aside while its replacement lands.
    Old,
    /// A folder being deleted (renamed first, so a scan never half-sees it).
    Deleting,
}

impl Aside {
    fn tag(self) -> &'static str {
        match self {
            Aside::Staging => ".staging-",
            Aside::Old => ".old-",
            Aside::Deleting => ".deleting-",
        }
    }
}

/// A fresh aside name for content folder `name` (see [`Aside`]), owned by
/// this process.
pub fn aside_name(name: &str, kind: Aside) -> String {
    format!(".{name}{}{}-{}", kind.tag(), std::process::id(), super::new_nonce())
}

/// What [`aside_name`] made `file_name` from: (the content folder's name,
/// the kind, the owning process — `None` for a name without one).
fn parse_aside(file_name: &str) -> Option<(&str, Aside, Option<u32>)> {
    let rest = file_name.strip_prefix('.')?;
    [Aside::Staging, Aside::Old, Aside::Deleting]
        .into_iter()
        .filter_map(|kind| {
            let at = rest.rfind(kind.tag())?;
            let name = &rest[..at];
            let tail = &rest[at + kind.tag().len()..];
            let pid = tail.split_once('-').and_then(|(pid, _)| pid.parse().ok());
            (!name.is_empty() && !tail.is_empty()).then_some((name, kind, pid))
        })
        .next()
}

/// Clear what interrupted installs and deletes left in a library folder
/// `dir` ([`Aside`] folders), and finish an interrupted replace:
///
/// - an aside folder of a process that is gone — or of this process when
///   `own_idle` (the caller has no install or delete running) — is
///   removed, except
/// - an `Old` folder whose content folder is missing: the process died
///   between [`promote`]'s two renames, so the old install is renamed back.
///
/// A folder of a live process is left alone. Never touches anything not in
/// the aside shape.
pub fn sweep_install_leftovers(dir: &Path, own_idle: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let own = std::process::id();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let Some((name, kind, pid)) = parse_aside(&file_name) else {
            continue;
        };
        let owner_gone = match pid {
            Some(pid) if pid == own => own_idle,
            Some(pid) => !crate::drumkit_library::pid_alive(pid),
            // Written before aside names carried a pid: nobody's now.
            None => own_idle,
        };
        let path = entry.path();
        if !owner_gone || !path.is_dir() {
            continue;
        }
        let dest = dir.join(name);
        if kind == Aside::Old && !dest.exists() && super::is_safe_id(name) {
            match std::fs::rename(&path, &dest) {
                Ok(()) => {
                    tracing::info!("restored {} from an interrupted replace", dest.display());
                    continue;
                }
                Err(e) => tracing::warn!("restore {}: {e}", dest.display()),
            }
        }
        if let Err(e) = std::fs::remove_dir_all(&path) {
            tracing::warn!("remove install leftover {}: {e}", path.display());
        }
    }
}
