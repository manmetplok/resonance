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

/// One preset as the host's preset surfaces list it: the browser a plugin
/// panel's bar opens, and the media browser's Presets tab (slice P6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPresetRow {
    pub plugin_id: String,
    pub plugin_name: String,
    pub id: String,
    pub name: String,
    pub source: resonance_control::methods::plugin_preset::PluginPresetSource,
    pub category: Option<String>,
    pub favorite: bool,
}

/// A press on a Presets-tab row. It is a drag only once the pointer has
/// moved past [`PresetDrag::THRESHOLD`]; a release before that is a click
/// and disarms it without dropping anything.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetDrag {
    pub row: HostPresetRow,
    /// Where the pointer was first seen after the press.
    pub origin: Option<iced::Point>,
    pub moved: bool,
}

impl PresetDrag {
    /// Pointer travel, in logical pixels, that turns a press into a drag.
    pub const THRESHOLD: f32 = 4.0;
}

/// A host preset list: its query and the rows it matched, recomputed in
/// `update` (never per frame) whenever the query or the library changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostPresetList {
    /// Free text with the library's search syntax (`is:fav`, `cat:…`).
    pub query: String,
    pub favorites_only: bool,
    /// Only this plugin's presets (a CLAP id); `None` lists every plugin.
    pub plugin: Option<String>,
    pub rows: Vec<HostPresetRow>,
    pub selected: Option<usize>,
    /// Bumped by every re-query: the key of the rows' `lazy` region.
    pub generation: u64,
}

/// The sound a plugin had before its first audition: what a revert puts
/// back and what the commit's undo entry returns to (§6.7). `state` is the
/// plugin's full state, saved by the engine under the plugin's lock just
/// before the first audition loaded (`capture` token `token`); it fills in
/// asynchronously.
#[derive(Debug, Clone)]
pub struct AuditionOrigin {
    pub values: Vec<(u32, f64)>,
    pub identity: Option<SlotPresetIdentity>,
    pub state: crate::undo::snapshot::LateBlob,
    pub token: u64,
}

/// The preset browser opened from a plugin panel's bar, over one plugin
/// instance. Clicking a row auditions it (unrecorded); keeping records one
/// undo entry from the origin; Esc reverts.
#[derive(Debug, Clone)]
pub struct HostPresetBrowser {
    pub instance_id: PluginInstanceId,
    pub plugin_name: String,
    pub list: HostPresetList,
    pub origin: Option<AuditionOrigin>,
    /// The preset last auditioned: what Keep keeps, even once a search
    /// has filtered its row out of the list.
    pub auditioned: Option<HostPresetRow>,
}

/// A plugin the media browser's Presets tab can narrow to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetPluginChoice {
    /// `None` is "every plugin".
    pub plugin_id: Option<String>,
    pub name: String,
}

impl std::fmt::Display for PresetPluginChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// A favourite preset an add picker offers ("with preset…").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetAddPick {
    pub plugin: resonance_audio::types::ScannedPlugin,
    pub preset_id: String,
    pub preset_name: String,
    pub source: resonance_control::methods::plugin_preset::PluginPresetSource,
}

impl std::fmt::Display for PresetAddPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} \u{2014} {}", self.plugin.name, self.preset_name)
    }
}

/// See `PresetState::pending_after`.
#[derive(Debug, Clone, Default)]
pub(crate) struct PendingAfter {
    pub instance_id: PluginInstanceId,
    pub slots: std::sync::Arc<std::sync::Mutex<Vec<crate::undo::snapshot::LateBlob>>>,
    pub superseded: bool,
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
    /// What each plugin's preset-discovery factory listed (slice P8), by
    /// CLAP id; registered with the library as read-only factory presets.
    pub discovered:
        std::collections::HashMap<String, Vec<resonance_audio::types::DiscoveredPreset>>,
    /// A press on a Presets-tab row, which becomes a drag onto a track
    /// header once the pointer moves (slice P8).
    pub dragging: Option<PresetDrag>,
    /// Set while a ◀ / ▶ step applies its load: the state load is parked
    /// in `pending_step_state` instead of sent.
    pub(crate) debounce_state_load: bool,
    /// A step run's parked state load: data, capture token, last step.
    pub(crate) pending_step_state:
        std::collections::HashMap<PluginInstanceId, (Vec<u8>, Option<u64>, std::time::Instant)>,
    /// When a visible preset list last re-queried the library.
    pub(crate) lists_polled: Option<std::time::Instant>,
    /// The preset browser over one plugin, when open (a root overlay).
    pub host_browser: Option<HostPresetBrowser>,
    /// The media browser's Presets tab.
    pub media_presets: HostPresetList,
    /// The Presets tab's plugin choices ("All plugins" first), rebuilt on
    /// a plugin scan so the pick list does not rebuild them per frame.
    pub media_plugin_choices: std::rc::Rc<[PresetPluginChoice]>,
    /// Favourite presets of the scanned effects / instruments, for the add
    /// pickers' "with preset…" list; rebuilt on a scan and when a star
    /// changes from the host.
    pub fx_favorite_picks: std::rc::Rc<[PresetAddPick]>,
    pub instrument_favorite_picks: std::rc::Rc<[PresetAddPick]>,
    /// The late undo slot the last recorded preset load's snapshot holds,
    /// for that load to fill (`undo::attach_preset_capture`).
    pub(crate) next_capture:
        Option<(PluginInstanceId, crate::undo::snapshot::LateBlob)>,
    /// A load that must capture into this token (an audition's origin).
    pub(crate) forced_capture: Option<u64>,
    /// A kept audition: its undo slot takes the origin's state rather than
    /// a capture (the engine plays the audition by now).
    pub(crate) capture_from: Option<(crate::undo::snapshot::LateBlob, u64)>,
    /// Captures in flight: the slots each token fills.
    pub(crate) pending_captures:
        std::collections::HashMap<u64, Vec<crate::undo::snapshot::LateBlob>>,
    /// A revert that happened before its origin's capture arrived: load
    /// the capture into this instance when it does.
    pub(crate) revert_on_capture: std::collections::HashMap<u64, PluginInstanceId>,
    pub(crate) capture_seq: u64,
    /// Captures whose "after" half (the state the load left) is still
    /// owed, by token: the instance, the late slots of snapshots taken
    /// before it arrived (shared behind a lock so `snapshot_for_undo(&self)`
    /// can register one), and whether an undo / redo restore has run since
    /// (then the arriving state is no longer the live one).
    pub(crate) pending_after: std::collections::HashMap<u64, PendingAfter>,
    /// A preset to load onto a plugin that is being added, once its
    /// `PluginAdded` echo brings the param list: `(clap id, preset id,
    /// source)` by instance (a `preset` on `*.add_effect`, "with preset…"
    /// in the add pickers — slice P6).
    pub pending_plugin_presets: std::collections::HashMap<
        PluginInstanceId,
        (String, String, resonance_control::methods::plugin_preset::PluginPresetSource),
    >,
    /// Root the plugin-preset directories are read from and written to,
    /// when it is not the user's real data directory. A test seam.
    pub plugin_preset_root: Option<std::path::PathBuf>,
    /// The shared marks store (favourites, personal tags, recents) the
    /// plugin preset library reads, opened on first use
    /// (`crate::plugin_preset_library::marks`).
    /// The preset library, built on first use for `plugin_preset_root`
    /// (`crate::plugin_preset_library::library`); reset when the root moves.
    pub library_cache:
        std::sync::OnceLock<std::sync::Arc<resonance_plugin::presets::PresetLibrary>>,
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
