//! Session state for the missing-plugin warning (ba doc #275 P5, todo
//! #1309).
//!
//! The durable fact "this slot has no plugin behind it" lives on the
//! slot, as
//! [`PluginSlotState::availability`](crate::state::PluginSlotState::availability).
//! Held here is only the *session* question of whether the user has been
//! told: a project opened on a machine without one of its plugins raises
//! the warning once, and dismissing it must not be undone by the next
//! failure in the same load.
//!
//! Nothing here is persisted or undoable — it is the twin of
//! [`RelinkState`](crate::state::relink::RelinkState), which does the
//! same job for missing media files, and the two are deliberately shaped
//! alike.
//!
//! The *contents* of the warning are derived, never stored: the list
//! comes from walking the chains for missing slots
//! ([`Resonance::missing_plugin_slots`](crate::Resonance::missing_plugin_slots)).
//! A stored list would be a second copy of the truth, and would go stale
//! the moment a slot was replaced or removed while the modal was open.

use resonance_audio::types::PluginInstanceId;

use crate::state::{PluginLocator, PluginSlotState};
use crate::Resonance;

/// One slot the engine could not fill, resolved against the chain it
/// sits in — everything the warning list, the inspector badge and the
/// control API's `status` need to say which plugin is missing and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPluginSlot {
    pub instance_id: PluginInstanceId,
    /// Which chain carries the slot.
    pub owner: PluginLocator,
    /// The chain's name as the user sees it — a track or bus name, or
    /// `"Master"`.
    pub owner_label: String,
    /// 0-based position in that chain, counting missing slots: the
    /// position the plugin keeps, and the one a replacement inherits.
    pub slot: usize,
    /// The plugin's name as the PROJECT recorded it. The catalog cannot
    /// supply one — that is the whole problem — so this is the only
    /// human-readable name a missing plugin has.
    pub plugin_name: String,
    pub clap_plugin_id: String,
    pub clap_file_path: String,
    /// Why the engine could not load it.
    pub reason: String,
}

impl Resonance {
    /// Every slot in the project with no plugin behind it, in chain
    /// order: tracks (in mixer order), then busses, then master.
    ///
    /// Derived on demand rather than accumulated. A stored list would
    /// have to be pruned by every remove, replace, relocate, undo and
    /// project clear; walking the chains cannot go stale, and the walk
    /// is over a handful of plugins per chain.
    pub(crate) fn missing_plugin_slots(&self) -> Vec<MissingPluginSlot> {
        let mut out = Vec::new();
        for track in self.sorted_tracks() {
            collect_missing(
                &track.plugins,
                PluginLocator::Track(track.id),
                &track.name,
                &mut out,
            );
        }
        for bus in self.sorted_busses() {
            collect_missing(
                &bus.plugins,
                PluginLocator::Bus(bus.id),
                &bus.name,
                &mut out,
            );
        }
        collect_missing(
            &self.master_plugins,
            PluginLocator::Master,
            "Master",
            &mut out,
        );
        out
    }

    /// True when any chain carries a slot the engine could not fill.
    pub(crate) fn has_missing_plugins(&self) -> bool {
        self.registry
            .tracks
            .iter()
            .any(|t| t.plugins.iter().any(|p| p.availability.is_missing()))
            || self
                .registry
                .busses
                .iter()
                .any(|b| b.plugins.iter().any(|p| p.availability.is_missing()))
            || self
                .master_plugins
                .iter()
                .any(|p| p.availability.is_missing())
    }
}

fn collect_missing(
    chain: &[PluginSlotState],
    owner: PluginLocator,
    owner_label: &str,
    out: &mut Vec<MissingPluginSlot>,
) {
    for (slot, plugin) in chain.iter().enumerate() {
        let Some(reason) = plugin.availability.reason() else {
            continue;
        };
        out.push(MissingPluginSlot {
            instance_id: plugin.instance_id,
            owner,
            owner_label: owner_label.to_owned(),
            slot,
            plugin_name: plugin.plugin_name.clone(),
            clap_plugin_id: plugin.clap_plugin_id.clone(),
            clap_file_path: plugin.clap_file_path.clone(),
            reason: reason.to_owned(),
        });
    }
}

/// Whether the missing-plugin warning is on screen, and whether it has
/// already been dismissed for the currently-open project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissingPluginState {
    /// Whether the warning modal is currently shown.
    pub modal_open: bool,
    /// Whether the user has dismissed the warning for this project.
    ///
    /// Failures arrive one engine event at a time, so without this a
    /// dismissal during a load that is still reporting would be undone
    /// by the very next failure and the modal would appear to be
    /// un-closable.
    pub dismissed: bool,
}

impl MissingPluginState {
    /// A plugin failed to instantiate onto a slot. Raises the warning
    /// unless the user has already dismissed it for this project.
    pub fn note_failure(&mut self) {
        if !self.dismissed {
            self.modal_open = true;
        }
    }

    /// Close the warning and keep it closed for this project.
    pub fn dismiss(&mut self) {
        self.modal_open = false;
        self.dismissed = true;
    }

    /// Forget everything — called when a project is cleared, so the next
    /// one gets its own warning. A project whose plugins are all present
    /// simply never calls [`note_failure`](Self::note_failure).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Re-open the warning on demand (the mixer's missing-plugin chip),
    /// even after a dismissal.
    pub fn show(&mut self) {
        self.modal_open = true;
    }
}
