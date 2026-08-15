//! GUI-side mirror of the engine's sidechain (key) routing table.
//!
//! One entry per keyed plugin instance: a detector has exactly one
//! input, so a plugin can read at most one key source. The engine holds
//! the authoritative table (`engine::sidechain::SidechainRoutes`); this
//! is the app's copy of it, rebuilt from `SidechainRouteChanged` echoes
//! and — like [`AuxSendState`](super::AuxSendState) — seeded directly by
//! the project-load replay so the route is present the moment the load
//! returns rather than one event-pump tick later.
//!
//! **Why the app needs a mirror at all.** Before ba todo #1311 it did
//! not have one: `SetPluginSidechain` fired the engine command and kept
//! nothing, and the `SidechainRouteChanged` echo was consumed with a
//! `{}` arm. That is exactly why routes were not persisted — the save
//! path serializes app state, and no app state held the route. Nothing
//! could write down a duck that only existed inside the engine thread.
//!
//! **Pruning is one-sided here, unlike aux sends.** The engine already
//! drops routes on `RemoveTrack` / `RemoveBus` (source gone) and on
//! `RemovePlugin` (target gone), so the mirror only has to keep up — it
//! does not have to tell the engine. The one exception is a plugin taken
//! off a *bus* or the *master* chain: `RemovePluginFromBus` /
//! `RemovePluginFromMaster` do not drop the route engine-side, so
//! [`Resonance`](crate::Resonance) sends an explicit `ClearSidechainRoute`
//! when it prunes one of those. See `engine_events::plugins`.

use resonance_audio::types::{PluginInstanceId, SendSource, SidechainRoute};

/// GUI-side mirror of the engine's key-routing table.
#[derive(Debug, Default)]
pub struct SidechainState {
    /// Every live key route, at most one per keyed plugin instance.
    /// Insertion-ordered; project serialization sorts by target id so the
    /// on-disk order does not depend on the order echoes arrived in.
    pub routes: Vec<SidechainRoute>,
}

impl SidechainState {
    /// Insert or replace the route keying `route.plugin`.
    ///
    /// Replacement rather than append is the whole contract: a detector
    /// has one input, and the engine's table is a `HashMap` keyed by
    /// plugin instance, so a second route onto the same plugin *replaces*
    /// the first there. A mirror that appended would show two key sources
    /// for one detector and write both to the project file.
    pub fn upsert(&mut self, route: SidechainRoute) {
        match self.routes.iter_mut().find(|r| r.plugin == route.plugin) {
            Some(existing) => *existing = route,
            None => self.routes.push(route),
        }
    }

    /// Drop `plugin`'s route. `true` when one was actually present.
    pub fn clear_plugin(&mut self, plugin: PluginInstanceId) -> bool {
        let before = self.routes.len();
        self.routes.retain(|r| r.plugin != plugin);
        self.routes.len() != before
    }

    /// The route keying `plugin`, if any.
    pub fn route_for(&self, plugin: PluginInstanceId) -> Option<SidechainRoute> {
        self.routes.iter().find(|r| r.plugin == plugin).copied()
    }

    /// Drop every route keyed *from* `source`, returning the plugin
    /// instances that lost their key.
    ///
    /// Called when a track or bus is deleted. A route is an edge, so it
    /// stops meaning anything once the source is gone — and leaving it in
    /// the mirror is durable damage rather than a cosmetic wart: the
    /// mirror is what save serializes, so a dangling route would be
    /// written to the project file, refused by the loader on reopen, and
    /// rewritten by every save after that. (The same failure the aux-send
    /// graph hit in ba todo #1269.)
    pub fn drop_routes_from_source(&mut self, source: SendSource) -> Vec<PluginInstanceId> {
        let dropped: Vec<PluginInstanceId> = self
            .routes
            .iter()
            .filter(|r| r.source == source)
            .map(|r| r.plugin)
            .collect();
        self.routes.retain(|r| r.source != source);
        dropped
    }

    /// Forget every route. Used when a project load wipes the registry:
    /// `ClearAll` empties the engine's table without echoing a
    /// per-route change, so the mirror has to be emptied here or the
    /// newly loaded project inherits keys from the previous one.
    pub fn clear(&mut self) {
        self.routes.clear();
    }
}
