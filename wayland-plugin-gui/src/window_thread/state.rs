//! State held by the editor thread, mutated by SCTK dispatch handlers.

use std::time::Instant;

use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::window::{DecorationMode, Window};
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
    /// `Some(when)` while a `wl_surface.frame()` callback requested at
    /// `when` is still outstanding. Painting is gated on this being
    /// `None`: the compositor tells us when it wants the next frame, so
    /// we never render faster than it presents and render nothing at
    /// all while it withholds callbacks (occluded surface). The
    /// timestamp lets the event loop treat a long-overdue callback as
    /// lost instead of freezing the GUI (see `FRAME_CALLBACK_STALL`).
    pub(super) frame_callback_pending: Option<Instant>,
    pub(super) close_requested: bool,
    pub(super) input: InputState,
    pub(super) pending_events: Vec<egui::Event>,
    pub(super) egui_ctx: egui::Context,
    /// Decoration mode the compositor negotiated for the toplevel, read back
    /// from `WindowConfigure::decoration_mode` on every configure. We request
    /// [`DecorationMode::Server`] (SSD) but the compositor may force
    /// [`DecorationMode::Client`] (or never offer SSD at all), in which case
    /// the paint path draws a CSD fallback frame so the window is still
    /// usable. Defaults to `Server` until the first configure arrives.
    pub(super) decoration_mode: DecorationMode,
    /// Whether the runtime prefers server-side decorations (the `WPG_FORCE_SSD`
    /// opt-in). When `false` (the default), the compositor is asked not to
    /// decorate and `decoration_mode` is pinned to [`DecorationMode::Client`]
    /// so the runtime always draws its own frame with a working close button —
    /// even on wlroots compositors whose SSD has no close affordance. When
    /// `true`, the negotiated `WindowConfigure::decoration_mode` is honoured and
    /// CSD is drawn only when the compositor forces client-side mode.
    pub(super) prefer_server: bool,
    /// Window title, mirrored here so the CSD titlebar can render it without
    /// reaching back into `EditorOptions`.
    pub(super) title: String,
}

impl State {
    /// Whether the editor must draw its own client-side decoration frame:
    /// true when the compositor negotiated [`DecorationMode::Client`].
    pub(super) fn needs_csd(&self) -> bool {
        matches!(self.decoration_mode, DecorationMode::Client)
    }

    /// The integer buffer scale used for rendering.
    ///
    /// Single source of truth that keeps the three places a scale
    /// appears in agreement: `wl_surface.set_buffer_scale` (core
    /// protocol, integers only), the wl_egl_window's physical size, and
    /// egui's `pixels_per_point`. `scale` only ever holds whole numbers
    /// today (it comes from `CompositorHandler::scale_factor_changed`,
    /// an `i32`); the rounding here is defensive so a future fractional
    /// source still yields one consistent integer everywhere. True
    /// fractional rendering would need `wp-fractional-scale-v1` plus
    /// `wp_viewport` instead of `set_buffer_scale` — not wired through.
    pub(super) fn buffer_scale(&self) -> i32 {
        self.scale.max(1.0).round() as i32
    }

    /// Physical (buffer) size in pixels: logical size x [`Self::buffer_scale`].
    /// Use this for both the EGL surface size and the GL viewport so they
    /// cannot drift apart.
    pub(super) fn physical_size(&self) -> (i32, i32) {
        let s = self.buffer_scale();
        (self.size.0 as i32 * s, self.size.1 as i32 * s)
    }
}
