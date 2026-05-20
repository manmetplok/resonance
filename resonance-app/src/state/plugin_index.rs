//! Plugin-instance side-index and the `with_plugin_mut` accessor that
//! relies on it.
//!
//! `Resonance::plugin_index` maps every live `PluginInstanceId` to the
//! container that currently owns it (a track, a bus, or master).
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
        let result = match self.plugin_index.get(&instance_id).copied() {
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
        self.plugin_index.insert(instance_id, locator);
    }

    /// Drop `instance_id`'s side-index entry. Call after removing a
    /// slot, or for every instance under a track/bus that is being
    /// removed wholesale.
    pub(crate) fn remove_plugin_index(&mut self, instance_id: PluginInstanceId) {
        self.plugin_index.remove(&instance_id);
    }

    /// Recompute the entire `plugin_index` from `registry.tracks`,
    /// `registry.busses`, and `master_plugins`. Used after a full
    /// project replay or demo seed where the state is repopulated
    /// wholesale.
    pub(crate) fn rebuild_plugin_index(&mut self) {
        self.plugin_index.clear();
        for track in &self.registry.tracks {
            for p in &track.plugins {
                self.plugin_index
                    .insert(p.instance_id, PluginLocator::Track(track.id));
            }
        }
        for bus in &self.registry.busses {
            for p in &bus.plugins {
                self.plugin_index
                    .insert(p.instance_id, PluginLocator::Bus(bus.id));
            }
        }
        for p in &self.master_plugins {
            self.plugin_index
                .insert(p.instance_id, PluginLocator::Master);
        }
    }
}
