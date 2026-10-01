//! Where per-user marks (favourite, personal tags, recents) reach the
//! preset library (plugin-preset-library.md §4.1, §4.5).
//!
//! The store is the shared `resonance_common::library_marks`: one
//! `marks.json` for every library kind, keyed `plugin-preset:<clap id>:<preset
//! id>` for presets. The library reads it through [`MarksSource`], which
//! [`SharedMarks`] implements; a library with no store installed reads
//! [`NoMarks`]. Marks are read at query time and never cached in the index,
//! so they cannot go stale there; [`MarksSource::refresh`] is the hook that
//! picks up another process's write (the library calls it before every
//! query) and [`MarksSource::generation`] is what a browser keys its
//! cached view on, next to the index revision.

use resonance_common::library_marks::{self, kind, Marks, MarksError, SharedMarks};

/// Read (and, where the store allows, write) access to a marks store.
pub trait MarksSource: Send + Sync {
    /// The marks stored under `key` (see [`mark_key`]); the defaults when
    /// there are none.
    fn marks(&self, key: &str) -> Marks;

    /// The store's write counter, as last read. It moves whenever any
    /// process writes the store and a [`refresh`](Self::refresh) (or a
    /// write through this source) has seen it; a view cached on it is
    /// rebuilt when it moves. `0` for a store that never changes.
    fn generation(&self) -> u64 {
        0
    }

    /// Re-read the store if another writer changed it. Cheap when nothing
    /// changed (one `stat`). Returns whether anything changed.
    fn refresh(&self) -> bool {
        false
    }

    /// Apply `f` to the marks of `key` and persist it. Read-only sources
    /// refuse.
    fn update(&self, _key: &str, _f: &dyn Fn(&mut Marks)) -> Result<Marks, String> {
        Err("this library has no marks store".to_string())
    }

    /// Tag completion across every library kind (used tags first, then the
    /// seeded vocabulary).
    fn complete_tag(&self, _prefix: &str, _exclude: &[String], _limit: usize) -> Vec<String> {
        Vec::new()
    }
}

/// The marks source of a library with no store: nothing is marked.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMarks;

impl MarksSource for NoMarks {
    fn marks(&self, _key: &str) -> Marks {
        Marks::default()
    }
}

impl MarksSource for SharedMarks {
    fn marks(&self, key: &str) -> Marks {
        SharedMarks::marks(self, key)
    }

    fn generation(&self) -> u64 {
        SharedMarks::generation(self)
    }

    fn refresh(&self) -> bool {
        SharedMarks::refresh(self)
    }

    fn update(&self, key: &str, f: &dyn Fn(&mut Marks)) -> Result<Marks, String> {
        SharedMarks::update(self, key, f).map_err(|e: MarksError| e.to_string())
    }

    fn complete_tag(&self, prefix: &str, exclude: &[String], limit: usize) -> Vec<String> {
        self.snapshot().complete_tag(prefix, exclude, limit)
    }
}

/// The mark kind for plugin presets.
pub const PLUGIN_PRESET_KIND: &str = kind::PLUGIN_PRESET;

/// The marks-store key of a plugin preset:
/// `plugin-preset:<clap id>:<preset id>` (§4.2).
pub fn mark_key(plugin_id: &str, preset_id: &str) -> String {
    library_marks::mark_key(kind::PLUGIN_PRESET, &format!("{plugin_id}:{preset_id}"))
}

/// `last_used` in the RFC 3339 spelling presets and the wire carry.
pub fn last_used_rfc3339(marks: &Marks) -> Option<String> {
    marks.last_used_rfc3339()
}
