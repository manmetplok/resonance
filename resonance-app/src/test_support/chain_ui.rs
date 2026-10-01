//! Test-support accessors for the inspector CHAIN rows' view state
//! (mixer-cleanup.md §3.2, S7) and the header colour palette.

use crate::message::Message;
use crate::state;
use crate::Resonance;
use resonance_audio::types::{PluginInstanceId, TrackId};

impl Resonance {
    /// Test-only: the CHAIN row whose ☰ menu is open.
    #[doc(hidden)]
    pub fn test_slot_menu(&self) -> Option<PluginInstanceId> {
        self.ui.mixer.slot_menu
    }

    /// Test-only: the slot the CHAIN add picker is replacing.
    #[doc(hidden)]
    pub fn test_replacing_slot(&self) -> Option<PluginInstanceId> {
        self.ui.mixer.replacing_slot
    }

    /// Test-only: the open "Save preset…" prompt.
    #[doc(hidden)]
    pub fn test_slot_preset_save(&self) -> Option<&state::SlotPresetSaveState> {
        self.ui.mixer.slot_preset_save.as_ref()
    }

    /// Test-only: the armed CHAIN-row drag.
    #[doc(hidden)]
    pub fn test_chain_drag(&self) -> Option<state::ChainDragState> {
        self.ui.mixer.chain_drag
    }

    /// Test-only: the track whose inspector colour palette is open.
    #[doc(hidden)]
    pub fn test_color_palette(&self) -> Option<TrackId> {
        self.ui.mixer.color_palette
    }

    /// Test-only: the instance and name of an armed plugin-preset save
    /// (waiting on the engine's state capture).
    #[doc(hidden)]
    pub fn test_pending_plugin_preset_save(&self) -> Option<(PluginInstanceId, String)> {
        self.presets
            .pending_plugin_preset_save
            .as_ref()
            .map(|p| (p.instance_id, p.name.clone()))
    }

    /// Test-only: the ☰ menu entries of `instance_id`'s CHAIN row, as
    /// the view builds them — label and message (`None` = disabled).
    #[doc(hidden)]
    pub fn test_slot_menu_entries(
        &self,
        instance_id: PluginInstanceId,
    ) -> Vec<(String, Option<Message>)> {
        use crate::view::mixer::picks::PluginOwner;
        let Some((locator, index)) = crate::update::plugin_replace::locate_slot(self, instance_id)
        else {
            return Vec::new();
        };
        let (owner, chain) = match locator {
            state::PluginLocator::Track(id) => (
                PluginOwner::Track(id),
                self.registry
                    .tracks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.plugins.as_slice()),
            ),
            state::PluginLocator::Bus(id) => (
                PluginOwner::Bus(id),
                self.registry
                    .busses
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| b.plugins.as_slice()),
            ),
            state::PluginLocator::Master => {
                (PluginOwner::Master, Some(self.master.plugins.as_slice()))
            }
        };
        let Some(chain) = chain else {
            return Vec::new();
        };
        crate::view::mixer::inspector::chain::slot_menu_entries(
            self,
            owner,
            &chain[index],
            index,
            chain.len(),
        )
        .into_iter()
        .map(|(label, m)| (label.to_owned(), m))
        .collect()
    }
}
