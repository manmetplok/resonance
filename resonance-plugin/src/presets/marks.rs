//! Where per-user marks (favourite, personal tags, recents) plug into the
//! preset index (plugin-preset-library.md §4.1, §4.5).
//!
//! **Integration seam.** The marks store itself is
//! `resonance_common::library_marks`, shared with the NAM model library
//! and built on another branch. The preset library only needs to *read*
//! it while building query results, so it depends on this trait rather
//! than on the store: round 2 implements [`MarksSource`] for the shared
//! store and installs it with
//! [`PresetLibrary::set_marks`](super::PresetLibrary::set_marks). Until
//! then every library reads [`NoMarks`], which is the truthful answer for
//! a build with no marks store.
//!
//! Writes (star, personal tags, `last_used`) go through the store's own
//! API, not through this trait: the preset library never mutates marks.

/// The marks on one asset. Every field at its default means "no mark".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetMarks {
    pub favorite: bool,
    /// Personal tags. Merged with the preset's own content tags in query
    /// results; on factory presets these are the only editable tags.
    pub tags: Vec<String>,
    /// RFC 3339; written only for user picks (§4.5), never restores.
    pub last_used: Option<String>,
    pub use_count: u32,
}

/// Read access to a marks store.
pub trait MarksSource: Send + Sync {
    /// The marks stored under `key` (see [`mark_key`]); default marks
    /// when there are none.
    fn marks(&self, key: &str) -> PresetMarks;

    /// A counter the store bumps on every write. Part of the index
    /// freshness fingerprint, so a star set in another process is picked
    /// up by the next poll. `0` for a store that never changes.
    fn generation(&self) -> u64 {
        0
    }
}

/// The marks source of a build with no marks store: nothing is marked.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMarks;

impl MarksSource for NoMarks {
    fn marks(&self, _key: &str) -> PresetMarks {
        PresetMarks::default()
    }
}

/// The mark kind for plugin presets.
pub const PLUGIN_PRESET_KIND: &str = "plugin-preset";

/// The marks-store key of a plugin preset:
/// `plugin-preset:<clap id>:<preset id>` (§4.2; the NAM spec's
/// `<kind>:<id>` scheme).
pub fn mark_key(plugin_id: &str, preset_id: &str) -> String {
    format!("{PLUGIN_PRESET_KIND}:{plugin_id}:{preset_id}")
}
