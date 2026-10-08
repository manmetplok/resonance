//! What a rescan compares to decide a scanned bank is unchanged and its
//! [`InstalledBank`](super::InstalledBank) can be kept as-is — the parsed
//! configs, the inventory and the lazily-read English dictionary (TIGER's
//! ~11k entries) along with it.
//!
//! A [`BankStamp`] is the modification time and length of every file the
//! scan read (the acoustic config, the phoneme list, the character file,
//! `languages.json`, the English dictionary, the vocoder config), the
//! models a render loads (acoustic, vocoder and the bank's own, with their
//! configs — whose digests the render keys hash), plus the
//! bank folder and its immediate subfolders, whose times move when a file
//! is added or removed in them — a dictionary or a vocoder dropped in
//! later. Reading it is a few `stat` calls, against parsing YAML and JSON
//! for every bank on every `vocal.voicebanks`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One file's (or folder's) modification time and length, `None` when it
/// does not exist (or cannot be read).
type FileStamp = Option<(SystemTime, u64)>;

/// The stamps of the files a scan of one bank read, in a fixed order,
/// and the bank folder's subfolders by name — a folder added in the same
/// clock tick as the scan does not move a coarse folder time, but it does
/// change the list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct BankStamp {
    root: PathBuf,
    subdirs: Vec<PathBuf>,
    files: Vec<(PathBuf, FileStamp)>,
}

/// The paths a bank's stamp covers.
pub(super) struct StampedPaths<'a> {
    pub root: &'a Path,
    pub acoustic_config: &'a Path,
    pub phoneme_dict: &'a Path,
    pub english_dict: Option<&'a Path>,
    pub vocoder_config: Option<&'a Path>,
    /// The model files (and their configs) a render loads.
    pub model_files: &'a [PathBuf],
}

impl BankStamp {
    /// Stamp `paths` as they are on disk now.
    pub(super) fn read(paths: &StampedPaths<'_>) -> Self {
        let subdirs = subdirs(paths.root);
        let mut files: Vec<PathBuf> = vec![paths.root.to_path_buf()];
        files.extend(subdirs.iter().cloned());
        let acoustic_dir = paths.acoustic_config.parent().unwrap_or(paths.root);
        for dir in [paths.root, acoustic_dir] {
            files.push(dir.to_path_buf());
            for name in ["character.txt", "character.yaml", "languages.json"] {
                files.push(dir.join(name));
            }
        }
        files.push(paths.acoustic_config.to_path_buf());
        files.push(paths.phoneme_dict.to_path_buf());
        files.extend(paths.english_dict.map(Path::to_path_buf));
        files.extend(paths.vocoder_config.map(Path::to_path_buf));
        files.extend(paths.model_files.iter().cloned());
        files.sort();
        files.dedup();
        let files = files
            .into_iter()
            .map(|f| {
                let stamp = file_stamp(&f);
                (f, stamp)
            })
            .collect();
        Self {
            root: paths.root.to_path_buf(),
            subdirs,
            files,
        }
    }

    /// Whether the bank folder has the same subfolders and every stamped
    /// file still has the time and length it had.
    pub(super) fn is_current(&self) -> bool {
        subdirs(&self.root) == self.subdirs
            && self.files.iter().all(|(path, stamp)| file_stamp(path) == *stamp)
    }
}

fn file_stamp(path: &Path) -> FileStamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}
