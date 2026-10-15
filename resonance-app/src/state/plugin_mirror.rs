//! The app's mirror of live plugin instances that isn't already owned
//! per-track / per-bus (ARCH-06 A6-3).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about the plugin-instance mirror can take
//! `&PluginMirror` / `&mut PluginMirror` instead of the whole app.
//! Distinct from `registry.tracks[*].plugins` / `registry.busses[*].plugins`
//! (the chains themselves) and from `master.plugins`, which lives on
//! [`crate::state::MasterState`] (A6-3 last).

use resonance_audio::types::PluginInstanceId;

/// The app-side mirror of every live plugin instance: its cached opaque
/// state blob, the side-index from instance id to owning container, and
/// the id allocator every plugin add draws from.
#[derive(Debug, Clone, Default)]
pub struct PluginMirror {
    /// Cache of the most recently observed CLAP state blob per plugin
    /// instance. Populated from `PluginStateSaved` / `AllPluginStatesSaved`
    /// engine events and read into undo snapshots so restores can replay
    /// plugin internal state via `LoadPluginState`. Stale between
    /// refreshes — parameter values in snapshots always come from live
    /// GUI state instead.
    ///
    /// Also **seeded from the project file at load time** (see
    /// `update::project_io::reconcile::plugin_state::PluginState`), which is
    /// what keeps a slot whose `.clap` is missing from losing its opaque
    /// state: the engine can never report a blob for an instance it
    /// failed to create, so without the seed the first Save As wrote
    /// nothing for it and the settings were gone (ba doc #275, P5).
    /// Every writer of project state reads this map — the save collector
    /// via [`crate::update::project_io::plugin_states_for_save`], template
    /// capture, "save track as preset", and the undo snapshot — so the
    /// blob survives all of them.
    ///
    /// Blobs are `Arc<[u8]>` so an undo snapshot shares them instead of
    /// deep-copying KB–MB of NAM/IR/wavetable state per history entry
    /// (ARCH-09 A9-2); a blob is immutable once cached, and a refresh
    /// replaces the entry.
    pub state_cache: std::collections::HashMap<PluginInstanceId, std::sync::Arc<[u8]>>,

    /// Side-index mapping every live plugin instance to the slot that
    /// owns it (a track, a bus, or master). Kept in sync with
    /// `registry.tracks[*].plugins`, `registry.busses[*].plugins`, and
    /// `master.plugins` by `insert_plugin_index` / `remove_plugin_index`
    /// at each add/remove site, and wholesale via `rebuild_plugin_index`
    /// after seed / replay. Replaces the O(tracks × plugins) scan that
    /// `with_plugin_mut` did pre-index.
    pub index: std::collections::HashMap<PluginInstanceId, crate::state::ChainOwner>,

    /// Next plugin instance id the app will hand the engine — every
    /// plugin add now allocates from here (ARCH-04 D-1), not only a
    /// control-API add that has to report the slot it created without
    /// waiting for the `PluginAdded` echo (ba doc #273, todo #1234).
    /// Starts at 1: there is no engine-side counter left to stay above,
    /// so no base. See `Resonance::allocate_plugin_id`.
    pub next_id: PluginInstanceId,

    /// What each Resonance Drums instance last reported about the pads of
    /// the kit it plays (`com.resonance.kit-info`,
    /// `AudioEvent::PluginKitInfo`). Runtime only; the kit picker reads it.
    pub kit_info: std::collections::HashMap<PluginInstanceId, resonance_common::kit_info::KitInfo>,

    /// Each track instrument's output-port names as its `PluginAdded`
    /// reported them (index 0 is the main output), kept so the sub-track
    /// policy can run again when a parameter — the drums' `output_mode` —
    /// decides it later. Runtime only.
    pub output_ports: std::collections::HashMap<PluginInstanceId, Vec<String>>,

    /// Instances whose cached blob is known stale: the plugin changed
    /// state the blob carries (an editor kit pick, `kit_select`) and the
    /// `SavePluginState` that refreshes the cache is in flight (code
    /// review STATE2-07). An undo snapshot taken in that window holds the
    /// stale blob, and once the echo replaced the cache, undoing that
    /// unrelated edit pushed it back: the kit reverted with it. So such a
    /// snapshot also gets a late slot the echo fills, as a preset load's
    /// "after" capture does (`PresetState::pending_after`).
    pub(crate) owed_blobs: std::collections::HashMap<PluginInstanceId, OwedBlob>,
}

/// See [`PluginMirror::owed_blobs`].
///
/// Refreshes are matched to their echoes by request id (BUG-39): other
/// `SavePluginState`s are in flight too (a plugin add's seed, an editor
/// close), and counting echoes by position took an earlier one's — the
/// state from *before* the change — for the change's own.
#[derive(Debug, Clone, Default)]
pub(crate) struct OwedBlob {
    /// The refreshes still in flight, oldest first: one per change that
    /// made the blob stale.
    pub refreshes: Vec<OwedRefresh>,
    /// The late slots of snapshots taken while owed, each with the request
    /// id of the refresh whose echo fills it. Behind a lock so
    /// `snapshot_for_undo(&self)` can register one.
    pub slots: std::sync::Arc<
        std::sync::Mutex<Vec<(u64, crate::undo::snapshot::LateBlob)>>,
    >,
}

/// One refresh of an [`OwedBlob`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct OwedRefresh {
    /// The `SavePluginState` request id its echo carries.
    pub request: u64,
    /// An undo / redo restored since it was asked for: its echo describes
    /// a state the restore replaced, so it fills the slots waiting on it
    /// but not the live cache. Per refresh, so a change made after the
    /// restore owes a live echo again (BUG-39).
    pub superseded: bool,
}

impl OwedBlob {
    /// The refresh a snapshot taken now is owed: the newest one, unless a
    /// restore superseded it — the cache then holds the restored blob,
    /// which is the live state.
    pub fn owed_now(&self) -> Option<u64> {
        self.refreshes
            .last()
            .filter(|f| !f.superseded)
            .map(|f| f.request)
    }
}
