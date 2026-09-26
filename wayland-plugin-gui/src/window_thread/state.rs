//! State held by the editor thread, mutated by SCTK dispatch handlers.

use std::time::{Duration, Instant};

use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::window::{DecorationMode, Window};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::Connection;

use crate::input::InputState;
use crate::size::SharedSize;

/// How long a re-map after [`State::hide`] waits for the compositor's
/// configure before painting anyway.
///
/// xdg-shell says an unmapped toplevel is reset and must be configured
/// again before a buffer is attached, and wlroots-based compositors enforce
/// that (attaching early is a protocol error). Hyprland, though, keeps the
/// toplevel configured across the unmap and sends nothing until a buffer
/// arrives, so waiting for the configure would leave the window hidden
/// for good. A resetting compositor answers the initial commit within a
/// round trip — milliseconds — so this is generous for them and a short
/// delay on show for the others.
pub(super) const REMAP_CONFIGURE_WAIT: Duration = Duration::from_millis(200);

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
    /// The same size, published to the public [`crate::Editor`] handle
    /// every time it changes, so a compositor-driven resize is what the
    /// plugin (and through it the host) reports (ba todo #1337).
    pub(super) shared_size: SharedSize,
    pub(super) pending_size: Option<(u32, u32)>,
    pub(super) scale: f32,
    /// The host wants the window on screen (`Command::Show`, cleared by
    /// `Command::Hide`). Nothing is painted — and so nothing is mapped —
    /// while this is false; a new window starts hidden, per
    /// `Editor::new`'s "create but do not show".
    pub(super) visible: bool,
    /// A buffer is attached, i.e. the compositor has mapped the toplevel.
    /// Set by the first painted frame, cleared by [`State::hide`].
    pub(super) mapped: bool,
    /// [`State::hide`] unmapped the toplevel with a null buffer, which
    /// resets it to its just-created state; [`State::show`] has to redo
    /// the initial commit / configure round before painting again.
    pub(super) needs_remap: bool,
    /// `Some(deadline)` while a re-map waits for its configure; see
    /// [`REMAP_CONFIGURE_WAIT`].
    pub(super) remap_deadline: Option<Instant>,
    pub(super) running: bool,
    pub(super) configured: bool,
    pub(super) needs_redraw: bool,
    /// `Some(deadline)` while egui has asked for a repaint at a future
    /// instant (`request_repaint_after` at or beyond the immediate
    /// threshold — see [`plugin_gui_core::repaint`]). The event loop
    /// bounds its parking budget on this and converts it into
    /// `needs_redraw` once due; each painted frame re-plans it from that
    /// frame's `repaint_delay`.
    pub(super) repaint_at: Option<Instant>,
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
    /// Also mirrored for the re-map after a hide: unmapping discards every
    /// toplevel attribute (xdg-shell), so [`State::show`] sets them again.
    pub(super) app_id: String,
    pub(super) min_size: (u32, u32),
}

impl State {
    /// `Command::Show`. Paints on the next loop turn, which maps the
    /// window; after a [`State::hide`] it first re-runs the initial
    /// commit, and the paint waits for the configure that answers it (for
    /// at most [`REMAP_CONFIGURE_WAIT`]).
    pub(super) fn show(&mut self) {
        if self.visible {
            return;
        }
        self.visible = true;
        self.needs_redraw = true;
        // Whatever egui last knew about the pointer is stale: it left (or
        // never entered) while we were hidden.
        self.pending_events.push(egui::Event::PointerGone);
        if self.needs_remap {
            self.needs_remap = false;
            // The unmap returned the toplevel to the state right after
            // `get_toplevel`: title, app id and size limits are gone and
            // a buffer may only be attached after a fresh configure.
            self.window.set_title(&self.title);
            self.window.set_app_id(&self.app_id);
            self.window.set_min_size(Some(self.min_size));
            self.configured = false;
            self.remap_deadline = Some(Instant::now() + REMAP_CONFIGURE_WAIT);
            self.window.commit();
        }
    }

    /// `Command::Hide`. Really takes the window off screen: attaching a
    /// null buffer unmaps the toplevel (xdg-shell), where merely not
    /// painting left it mapped, frozen and still taking input (PLG-04).
    /// Input that arrives while hidden is dropped, not queued for a replay
    /// on the next show.
    pub(super) fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.pending_events.clear();
        if self.mapped {
            let surface = self.window.wl_surface();
            surface.attach(None, 0, 0);
            surface.commit();
            self.mapped = false;
            self.needs_remap = true;
            // An unmapped surface gets no frame callbacks; don't let the
            // one in flight hold up the first frame after a show.
            self.frame_callback_pending = None;
        }
    }

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
