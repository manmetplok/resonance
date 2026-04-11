//! Recent-projects store, persisted as JSON at
//! `dirs::config_dir()/resonance/recent.json`. Loaded once at app
//! startup and refreshed whenever a project is opened or saved.
//!
//! All I/O errors are swallowed (logged to stderr); a broken or
//! missing recent file must never prevent the app from starting.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RECENT: usize = 10;
const FILE_NAME: &str = "recent.json";
const APP_DIR: &str = "resonance";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub display_name: String,
    pub last_opened_secs: u64,
}

fn recent_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join(FILE_NAME))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn derive_display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn load() -> Vec<RecentEntry> {
    let Some(file) = recent_file_path() else {
        return Vec::new();
    };
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!("recent.json read failed: {e}");
            return Vec::new();
        }
    };
    match serde_json::from_slice::<Vec<RecentEntry>>(&bytes) {
        Ok(mut list) => {
            list.sort_by_key(|e| std::cmp::Reverse(e.last_opened_secs));
            list.truncate(MAX_RECENT);
            let before = list.len();
            list.retain(|e| e.path.exists());
            if list.len() != before {
                persist(&list);
            }
            list
        }
        Err(e) => {
            eprintln!("recent.json parse failed: {e}");
            Vec::new()
        }
    }
}

fn persist(list: &[RecentEntry]) {
    let Some(file) = recent_file_path() else {
        return;
    };
    if let Some(parent) = file.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("recent.json mkdir failed: {e}");
            return;
        }
    }
    match serde_json::to_vec_pretty(list) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&file, bytes) {
                eprintln!("recent.json write failed: {e}");
            }
        }
        Err(e) => eprintln!("recent.json serialize failed: {e}"),
    }
}

fn insert_pure(list: &mut Vec<RecentEntry>, path: &Path, now: u64) {
    list.retain(|e| e.path != path);
    list.insert(
        0,
        RecentEntry {
            path: path.to_path_buf(),
            display_name: derive_display_name(path),
            last_opened_secs: now,
        },
    );
    list.truncate(MAX_RECENT);
}

/// Insert `path` at the front of `list` (or move it there if it
/// already exists), truncate to `MAX_RECENT`, and persist to disk.
pub fn add(list: &mut Vec<RecentEntry>, path: &Path) {
    insert_pure(list, path, now_secs());
    persist(list);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, ts: u64) -> RecentEntry {
        RecentEntry {
            path: PathBuf::from(path),
            display_name: derive_display_name(Path::new(path)),
            last_opened_secs: ts,
        }
    }

    #[test]
    fn insert_dedupes_and_promotes_to_front() {
        let mut list = vec![entry("/a", 1), entry("/b", 2), entry("/c", 3)];
        insert_pure(&mut list, Path::new("/b"), 99);
        assert_eq!(list[0].path, PathBuf::from("/b"));
        assert_eq!(list[0].last_opened_secs, 99);
        assert_eq!(list.iter().filter(|e| e.path == PathBuf::from("/b")).count(), 1);
    }

    #[test]
    fn insert_truncates_to_max() {
        let mut list: Vec<RecentEntry> = (0..MAX_RECENT as u64)
            .map(|i| entry(&format!("/p{i}"), i))
            .collect();
        insert_pure(&mut list, Path::new("/new"), 100);
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0].path, PathBuf::from("/new"));
    }
}
