//! State held by the editor thread, mutated by SCTK dispatch handlers.

use std::time::Instant;

use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::reexports::calloop::LoopHandle;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::window::{DecorationMode, Window};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::{Connection, Proxy, QueueHandle};

use crate::input::InputState;
use crate::size::SharedSize;

/// `wl_display.sync` round trips a re-map after [`State::hide`] waits for
/// the compositor's configure before it concludes none is coming.
///
/// xdg-shell says an unmapped toplevel is reset and must be configured
/// again before a buffer is attached, and wlroots-based compositors enforce
/// that (attaching early is a protocol error). Hyprland, though, keeps the
/// toplevel configured across the unmap and sends nothing until a buffer
/// arrives, so waiting for the configure would leave the window hidden
/// for good.
///
/// So the wait is bounded by the compositor itself rather than by a
/// wall-clock guess (FU-M1c): the initial commit is followed by a sync,
/// and when that is answered, by a second one. A resetting compositor
/// queues its configure while handling the commit or, like wlroots, from
/// an idle callback right after that batch of requests — in both cases
/// before it can answer the second sync, which it only reads after the
/// first one's `done` reached us. Two answered round trips without a
/// configure therefore mean the compositor kept the toplevel configured,
/// and painting is safe. However slow or loaded the compositor, the paint
/// never overtakes a configure it was going to send.
pub(super) const REMAP_SYNC_ROUNDS: u8 = 2;

/// User data of a re-map's `wl_display.sync` callback; see
/// [`REMAP_SYNC_ROUNDS`].
pub(super) struct RemapSync {
    /// [`State::remap_generation`] when the sync was sent: an answer that
    /// belongs to an earlier hide/show cycle is ignored.
    pub(super) generation: u64,
    /// 1-based round number.
    pub(super) round: u8,
}

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
    /// True while a re-map waits for its configure; see
    /// [`REMAP_SYNC_ROUNDS`]. `configured` is false for exactly as long.
    pub(super) remap_pending: bool,
    /// Bumped by every re-map, to match sync answers to their cycle.
    pub(super) remap_generation: u64,
    /// Needed to send the re-map's `wl_display.sync` from [`State::show`].
    pub(super) qh: QueueHandle<State>,
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
    /// Needed to register the client-side key-repeat timer
    /// (`get_keyboard_with_repeat`) from `new_capability`, where only
    /// `&mut State` is available — the event loop that owns this
    /// handle is what runs that timer (PUX-10).
    pub(super) loop_handle: LoopHandle<'static, State>,
    /// Whether this surface currently has keyboard focus, from the
    /// `enter`/`leave` keyboard events (PUX-10). Read into
    /// `RawInput::focused` every frame instead of the hardcoded `true`
    /// it used to be — plain text entry (a param's typed-value field)
    /// behaves oddly in egui when the backend claims focus it doesn't
    /// have.
    pub(super) keyboard_focused: bool,
}

impl State {
    /// `Command::Show`. Paints on the next loop turn, which maps the
    /// window; after a [`State::hide`] it first re-runs the initial
    /// commit, and the paint waits for the configure that answers it (or
    /// for [`REMAP_SYNC_ROUNDS`] round trips proving none is coming).
    pub(super) fn show(&mut self) {
        if self.visible {
            return;
        }
        self.visible = true;
        self.needs_redraw = true;
        // Whatever egui last knew about the pointer is stale: it left (or
        // never entered) while we were hidden, and a button held across
        // the hide was released where we could not see it (FU-M1c).
        self.input.release_all(&mut self.pending_events);
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
            self.remap_pending = true;
            self.remap_generation += 1;
            self.window.commit();
            self.send_remap_sync(1);
        }
    }

    fn send_remap_sync(&self, round: u8) {
        let data = RemapSync {
            generation: self.remap_generation,
            round,
        };
        let _ = self.conn.display().sync(&self.qh, data);
    }

    /// The compositor answered one of the re-map's syncs.
    pub(super) fn remap_sync_done(&mut self, sync: &RemapSync) {
        if !self.remap_pending || sync.generation != self.remap_generation {
            return;
        }
        if self.configured {
            // The configure arrived; the wait is over.
            self.remap_pending = false;
        } else if sync.round < REMAP_SYNC_ROUNDS {
            self.send_remap_sync(sync.round + 1);
        } else {
            // No configure is coming: the compositor kept the toplevel
            // configured across the unmap (Hyprland). Paint.
            self.remap_pending = false;
            self.configured = true;
            self.needs_redraw = true;
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

impl Drop for State {
    /// Release the keyboard before the event loop drops. A repeat keyboard
    /// (`get_keyboard_with_repeat`, PUX-10) keeps a [`LoopHandle`] in its
    /// proxy's user data, which the connection owns, and the event loop
    /// owns the connection (its `WaylandSource`): a cycle. Left alone, the
    /// loop, the connection and so the window outlived the editor thread,
    /// and a window closed from the compositor stayed mapped. Releasing
    /// destroys the proxy, which drops that user data. `State` is declared
    /// after the event loop in `EditorThread::run_inner`, so this runs
    /// first on every exit path, while the connection is still live.
    fn drop(&mut self) {
        if let Some(k) = self.keyboard.take() {
            // `wl_keyboard.release` is a v3 request; an older seat has no
            // way to destroy the keyboard at all.
            if k.version() >= 3 {
                k.release();
            }
        }
    }
}
