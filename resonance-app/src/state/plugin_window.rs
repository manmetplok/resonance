//! The host-drawn generic plugin window (mixer-cleanup.md §4).
//!
//! "Open" always opens a window: a plugin with its own GUI opens its
//! floating editor (`PluginMessage::OpenPluginEditor`), and a plugin
//! without one — or one that is missing on this machine, or whose editor
//! failed to open — gets this in-app floating panel instead: the generic
//! parameter list and the preset bar, layered over the app between the
//! base view and the modal overlays. It is non-modal, so it gates no keys
//! except Esc, which closes it.
//!
//! One generic window at a time: opening another plugin's replaces it.

use resonance_audio::types::PluginInstanceId;

use crate::state::MixerUiState;

/// Where a freshly opened window lands, in window coordinates.
pub const PLUGIN_WINDOW_DEFAULT_POSITION: iced::Point = iced::Point::new(240.0, 150.0);

/// The open generic plugin window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PluginWindowState {
    /// The plugin whose parameters the window shows.
    pub instance_id: PluginInstanceId,
    /// Top-left corner of the window, in window coordinates.
    pub position: iced::Point,
    /// The title-bar drag in progress, if any.
    pub drag: Option<PluginWindowDragState>,
}

/// A title-bar drag. The press carries no cursor position (an iced
/// `mouse_area` press doesn't), so the grab offset is taken from the first
/// pointer move after it — a pixel or two of slack, never a jump.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PluginWindowDragState {
    /// Cursor position minus window position, once known.
    pub grab: Option<iced::Vector>,
}

impl PluginWindowState {
    pub fn new(instance_id: PluginInstanceId, position: iced::Point) -> Self {
        Self {
            instance_id,
            position,
            drag: None,
        }
    }
}

impl MixerUiState {
    /// The plugin the generic window shows, if one is open. This is "the
    /// selected plugin" for the preset commands and the media tab's
    /// double-click load.
    pub fn plugin_window_id(&self) -> Option<PluginInstanceId> {
        self.plugin_window.map(|w| w.instance_id)
    }

    /// Close the generic window if it shows `instance_id` — the slot was
    /// removed, replaced or reloaded away.
    pub fn close_plugin_window_for(&mut self, instance_id: PluginInstanceId) {
        if self.plugin_window_id() == Some(instance_id) {
            self.plugin_window = None;
        }
    }
}
