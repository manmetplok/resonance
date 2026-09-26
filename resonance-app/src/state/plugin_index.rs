//! Plugin-instance side-index and the `with_plugin_mut` accessor that
//! relies on it.
//!
//! `Resonance::plugin_mirror.index` maps every live `PluginInstanceId` to
//! the container that currently owns it (a track, a bus, or master).
//! `with_plugin_mut` uses the index to jump straight to the owning chain
//! instead of scanning every track / bus / master plugin vector — the
//! pre-index path was the dominant cost on projects with many plugins.
//!
//! The trio `insert_plugin_index` / `remove_plugin_index` /
//! `rebuild_plugin_index` keeps the index in sync. Each add / remove
//! site (in `engine_events/plugins.rs`) calls the appropriate single-
//! entry helper; `rebuild_plugin_index` is the wholesale variant used
//! after a project replay or demo seed where the entire state tree is
//! repopulated at once.
//!
//! Because this module already owns the "every live plugin instance,
//! wherever it lives" view, it is also where
//! [`Resonance::allocate_plugin_id`] lives — the app-side instance-id
//! allocator every plugin add (GUI, control API, presets, templates,
//! project-load replay) draws from (ARCH-04 D-1, ba doc #273, todo
//! #1234). There is no engine-side counter to stay clear of any more:
//! the engine only ever honours the id it is given, and refuses a
//! collision (`resonance-audio/src/engine/plugins.rs::handle_add_plugin`
//! and its bus/master siblings) rather than allocating around it.

use resonance_audio::types::PluginInstanceId;

use crate::state::{PluginLocator, PluginSlotState};
use crate::Resonance;

impl Resonance {
    /// Locate a plugin slot on any track, bus, or master by instance id
    /// and run `f` on it. Uses the `plugin_index` side-table to jump
    /// directly to the owning container; falls back to a full scan on
    /// index miss so a desynced index degrades to the old O(n) path
    /// instead of returning `None` (the `debug_assert` flags the bug).
    pub(crate) fn with_plugin_mut<R>(
        &mut self,
        instance_id: PluginInstanceId,
        f: impl FnOnce(&mut PluginSlotState) -> R,
    ) -> Option<R> {
        let result = match self.plugin_mirror.index.get(&instance_id).copied() {
            Some(PluginLocator::Track(track_id)) => self
                .registry
                .tracks
                .iter_mut()
                .find(|t| t.id == track_id)
                .and_then(|t| t.plugins.iter_mut().find(|p| p.instance_id == instance_id))
                .map(f),
            Some(PluginLocator::Bus(bus_id)) => self
                .registry
                .busses
                .iter_mut()
                .find(|b| b.id == bus_id)
                .and_then(|b| b.plugins.iter_mut().find(|p| p.instance_id == instance_id))
                .map(f),
            Some(PluginLocator::Master) => self
                .master_plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
                .map(f),
            None => self.with_plugin_mut_linear(instance_id, f),
        };
        debug_assert!(
            result.is_some(),
            "with_plugin_mut: no plugin with id {instance_id:?}"
        );
        result
    }

    /// Linear-scan fallback used when the side-index has no entry for
    /// `instance_id`. Kept as a safety net so a missing index entry only
    /// costs a scan, not a silent miss.
    fn with_plugin_mut_linear<R>(
        &mut self,
        instance_id: PluginInstanceId,
        f: impl FnOnce(&mut PluginSlotState) -> R,
    ) -> Option<R> {
        for track in &mut self.registry.tracks {
            if let Some(p) = track
                .plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            {
                return Some(f(p));
            }
        }
        for bus in &mut self.registry.busses {
            if let Some(p) = bus
                .plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            {
                return Some(f(p));
            }
        }
        self.master_plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
            .map(f)
    }

    /// Record `instance_id`'s owning container in the side-index. Call
    /// after pushing a `PluginSlotState` into a track / bus / master
    /// chain.
    pub(crate) fn insert_plugin_index(
        &mut self,
        instance_id: PluginInstanceId,
        locator: PluginLocator,
    ) {
        self.plugin_mirror.index.insert(instance_id, locator);
    }

    /// Drop `instance_id`'s side-index entry. Call after removing a
    /// slot, or for every instance under a track/bus that is being
    /// removed wholesale.
    pub(crate) fn remove_plugin_index(&mut self, instance_id: PluginInstanceId) {
        self.plugin_mirror.index.remove(&instance_id);
    }

    /// Allocate a plugin instance id, for EVERY plugin add — GUI, control
    /// API, presets, templates, project-load replay (ARCH-04 D-1). The
    /// id is sent to the engine as `AudioCommand::AddPlugin`/
    /// `AddPluginToBus`/`AddPluginToMaster`'s `id` field, which the
    /// engine honours unconditionally and refuses to reuse
    /// (`resonance-audio/src/engine/plugins.rs::handle_add_plugin` and
    /// its bus/master siblings reject a collision with
    /// `EngineErrorKind::Internal` rather than replacing the live
    /// instance).
    ///
    /// Before D-1 this only ran for control-API adds that had to report
    /// their id synchronously (ba doc #273, todo #1234), from a base
    /// (`CONTROL_PLUGIN_ID_BASE`) chosen to sit above the engine's own
    /// counter so a GUI add's engine-allocated id could never land here.
    /// Now that the engine has no allocator of its own left for plugins,
    /// that partition is gone: this is the ONLY plugin-id allocator in
    /// the app, so there is no neighbouring range to stay clear of, and
    /// it starts at 1.
    ///
    /// The in-use scan checks the chains themselves, not `plugin_mirror.index`
    /// (a cache — a stale one must never hand out a live id), so it is
    /// what actually keeps two calls from returning the same id even
    /// though the counter alone cannot: a project loaded with ids the
    /// counter hasn't caught up to yet is exactly the case that scan is
    /// for.
    pub(crate) fn allocate_plugin_id(&mut self) -> PluginInstanceId {
        let (registry, master) = (&self.registry, &self.master_plugins);
        super::ids::allocate_unused(&mut self.plugin_mirror.next_id, |id| {
            registry
                .tracks
                .iter()
                .any(|t| t.plugins.iter().any(|p| p.instance_id == id))
                || registry
                    .busses
                    .iter()
                    .any(|b| b.plugins.iter().any(|p| p.instance_id == id))
                || master.iter().any(|p| p.instance_id == id)
        })
    }

    /// Recompute the entire `plugin_mirror.index` from `registry.tracks`,
    /// `registry.busses`, and `master_plugins`. Used after a full
    /// project replay or demo seed where the state is repopulated
    /// wholesale.
    pub(crate) fn rebuild_plugin_index(&mut self) {
        self.plugin_mirror.index.clear();
        for track in &self.registry.tracks {
            for p in &track.plugins {
                self.plugin_mirror
                    .index
                    .insert(p.instance_id, PluginLocator::Track(track.id));
            }
        }
        for bus in &self.registry.busses {
            for p in &bus.plugins {
                self.plugin_mirror
                    .index
                    .insert(p.instance_id, PluginLocator::Bus(bus.id));
            }
        }
        for p in &self.master_plugins {
            self.plugin_mirror
                .index
                .insert(p.instance_id, PluginLocator::Master);
        }
    }
}
