//! State held by the editor thread, mutated by SCTK dispatch handlers.

use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::window::Window;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::Connection;

use crate::input::InputState;

// ---------------------------------------------------------------------------
// State: holds everything SCTK dispatch handlers mutate.
// ---------------------------------------------------------------------------

pub(super) struct State {
    pub(super) registry_state: RegistryState,
    pub(super) seat_state: SeatState,
    pub(super) output_state: OutputState,
    pub(super) window: Window,
    pub(super) conn: Connection,
    pub(super) keyboard: Option<WlKeyboard>,
    pub(super) pointer: Option<WlPointer>,
    pub(super) size: (u32, u32),
    pub(super) pending_size: Option<(u32, u32)>,
    pub(super) scale: f32,
    pub(super) visible: bool,
    pub(super) running: bool,
    pub(super) configured: bool,
    pub(super) needs_redraw: bool,
    pub(super) close_requested: bool,
    pub(super) input: InputState,
    pub(super) pending_events: Vec<egui::Event>,
    pub(super) egui_ctx: egui::Context,
}
