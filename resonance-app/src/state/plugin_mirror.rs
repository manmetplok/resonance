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
    pub index: std::collections::HashMap<PluginInstanceId, crate::state::PluginLocator>,

    /// Next plugin instance id the app will hand the engine — every
    /// plugin add now allocates from here (ARCH-04 D-1), not only a
    /// control-API add that has to report the slot it created without
    /// waiting for the `PluginAdded` echo (ba doc #273, todo #1234).
    /// Starts at 1: there is no engine-side counter left to stay above,
    /// so no base. See `Resonance::allocate_plugin_id`.
    pub next_id: PluginInstanceId,
}
