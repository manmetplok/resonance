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

/// How much of the title bar, measured from its left edge, always stays
/// inside the app window — enough to grab it and drag it back.
pub const PLUGIN_WINDOW_GRAB_WIDTH: f32 = 120.0;
/// Height of the title bar strip that always stays inside the app window.
pub const PLUGIN_WINDOW_GRAB_HEIGHT: f32 = 30.0;

impl PluginWindowState {
    pub fn new(instance_id: PluginInstanceId, position: iced::Point) -> Self {
        Self {
            instance_id,
            position,
            drag: None,
        }
    }

    /// `position` pulled inside `viewport` so that a grab strip of the
    /// title bar ([`PLUGIN_WINDOW_GRAB_WIDTH`] x
    /// [`PLUGIN_WINDOW_GRAB_HEIGHT`]) stays on screen: never above or
    /// left of the app window's edge, never past its right or bottom
    /// edge minus that strip.
    pub fn clamp(position: iced::Point, viewport: iced::Size) -> iced::Point {
        let max_x = (viewport.width - PLUGIN_WINDOW_GRAB_WIDTH).max(0.0);
        let max_y = (viewport.height - PLUGIN_WINDOW_GRAB_HEIGHT).max(0.0);
        iced::Point::new(position.x.clamp(0.0, max_x), position.y.clamp(0.0, max_y))
    }

    /// Whether a window at `position` still has its grab strip inside
    /// `viewport` (it would not move under [`Self::clamp`]).
    pub fn title_on_screen(position: iced::Point, viewport: iced::Size) -> bool {
        Self::clamp(position, viewport) == position
    }
}

impl MixerUiState {
    /// The plugin the generic window shows, if one is open. (The preset
    /// commands act on [`MixerUiState::focused_slot`], not on this: a
    /// plugin with its own GUI never opens the generic window.)
    pub fn plugin_window_id(&self) -> Option<PluginInstanceId> {
        self.plugin_window.map(|w| w.instance_id)
    }

    /// Close the generic window if it shows `instance_id`. The slot stays
    /// focused ([`MixerUiState::focused_slot`]).
    pub fn close_plugin_window_for(&mut self, instance_id: PluginInstanceId) {
        if self.plugin_window_id() == Some(instance_id) {
            self.plugin_window = None;
        }
    }

    /// The slot `instance_id` is gone (removed, replaced, its owner
    /// deleted): close its window and drop it as the focused slot, so
    /// neither the window nor a preset command can act on an id the
    /// engine may hand to the next plugin added.
    pub fn forget_plugin(&mut self, instance_id: PluginInstanceId) {
        self.close_plugin_window_for(instance_id);
        if self.focused_slot == Some(instance_id) {
            self.focused_slot = None;
        }
    }
}
