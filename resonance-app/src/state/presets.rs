//! Track- and plugin-preset state (ARCH-06 A6-2/A6-3).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about presets can take `&PresetState` / `&mut
//! PresetState` instead of the whole app. Distinct from
//! `crate::presets`, which defines the `TrackPreset` type and the
//! default/user preset load functions this struct's lists are seeded
//! from.

use crate::{PendingPluginPresetSave, PendingPresetSave};
use resonance_audio::types::{PluginInstanceId, TrackId};

/// The preset a plugin slot has loaded, and whether it was edited since
/// (slice P5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPresetIdentity {
    pub source: resonance_control::methods::plugin_preset::PluginPresetSource,
    pub id: String,
    pub name: String,
    pub modified: bool,
    /// The plugin reports its own identity and modified flag
    /// (`com.resonance.preset-session`); the host no longer guesses.
    pub reported: bool,
}

/// Track-preset and plugin-preset save/apply state.
#[derive(Debug, Clone, Default)]
pub struct PresetState {
    /// Built-in default track presets (baked into the binary).
    pub default_presets: Vec<crate::presets::TrackPreset>,
    /// User-saved track presets (loaded from disk on startup).
    pub user_presets: Vec<crate::presets::TrackPreset>,
    /// When set, the next `TrackAdded` / `InstrumentTrackAdded` engine
    /// event will apply this preset to the newly created track.
    pub pending_track_preset: Option<crate::presets::TrackPreset>,
    /// When set, the next `AllPluginStatesSaved` event will capture
    /// plugin states for this track and save it as a user preset under
    /// this name (ba todo #1303).
    pub pending_preset_save: Option<PendingPresetSave>,
    /// A `*.save_plugin_preset` waiting for the plugin to hand back its
    /// state (ba todo #1333).
    pub(crate) pending_plugin_preset_save: Option<PendingPluginPresetSave>,
    /// Each plugin slot's loaded preset, keyed by instance: reported by
    /// the plugin, else set by the host's own loads (slice P5).
    pub plugin_preset_identity: std::collections::HashMap<PluginInstanceId, SlotPresetIdentity>,
    /// Root the plugin-preset directories are read from and written to,
    /// when it is not the user's real data directory. A test seam.
    pub plugin_preset_root: Option<std::path::PathBuf>,
    /// The shared marks store (favourites, personal tags, recents) the
    /// plugin preset library reads, opened on first use
    /// (`crate::plugin_preset_library::marks`).
    pub library_marks:
        std::sync::OnceLock<std::sync::Arc<resonance_common::library_marks::SharedMarks>>,
    /// Plugin state blobs to apply as PluginAdded events arrive for a
    /// preset-created track. Tuple of (target track id, ordered list of
    /// state blobs matching the preset's plugin chain).
    pub pending_preset_plugin_states: Option<(TrackId, Vec<Option<Vec<u8>>>)>,
    /// Saved plugin-parameter overrides waiting for their plugin's
    /// `PluginAdded` event, keyed by plugin instance id.
    pub pending_plugin_param_overrides:
        std::collections::HashMap<PluginInstanceId, Vec<crate::project::ProjectPluginParam>>,
}
