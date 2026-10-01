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

    /// Test-only: the name of `instance_id`'s armed plugin-preset save
    /// (waiting on the engine's state capture), if one is armed.
    #[doc(hidden)]
    pub fn test_pending_plugin_preset_save(&self, instance_id: PluginInstanceId) -> Option<String> {
        self.presets
            .pending_plugin_preset_saves
            .get(&instance_id)
            .map(|p| p.name.clone())
    }

    /// Test-only: the id of the user preset named `name` (any case) of
    /// the plugin behind `instance_id`, if one exists.
    #[doc(hidden)]
    pub fn test_user_preset_id(&self, instance_id: PluginInstanceId, name: &str) -> Option<String> {
        let clap_id = self.plugin_slot(instance_id)?.clap_plugin_id.clone();
        crate::update::control::plugin_presets::bank_for(self, &clap_id)
            .list_user()
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .map(|p| p.id.clone())
    }

    /// Test-only: the widget id of the CHAIN preset prompt's name field.
    #[doc(hidden)]
    pub fn test_preset_name_input_id() -> iced::widget::Id {
        crate::view::mixer::inspector::chain::preset_name_input_id()
    }

    /// Test-only: the track whose CHAIN `+ Add instrument` picker is cued.
    #[doc(hidden)]
    pub fn test_instrument_picker_cue(&self) -> Option<TrackId> {
        self.ui.mixer.instrument_picker_cue
    }

    /// Test-only: what a CHAIN row's `↗` carries and how it is tinted —
    /// the tint is the only feedback that the slot's window is open, and
    /// `iced_test` cannot read a colour.
    #[doc(hidden)]
    pub fn test_chain_open_toggle(
        &self,
        instance_id: PluginInstanceId,
    ) -> Option<(Message, iced::Color)> {
        let slot = self.plugin_slot(instance_id)?;
        Some(crate::view::mixer::inspector::chain::open_toggle_spec(self, slot))
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
