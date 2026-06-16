//! SCTK delegate trait impls for [`State`] and the `delegate_*!` macro wiring.

use std::num::NonZeroU32;

use smithay_client_toolkit::compositor::CompositorHandler;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers as SctkModifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::xdg::window::{
    DecorationMode, Window, WindowConfigure, WindowHandler,
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use super::state::State;

// --- SCTK delegate impls ---------------------------------------------------

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &WlSurface,
        new_factor: i32,
    ) {
        self.scale = new_factor as f32;
        self.input.set_scale(self.scale);
        // Keep the committed buffer scale in step with the factor we
        // render at — `EglContext::new` only sets it once at startup.
        // Double-buffered state; applied on the next commit, which the
        // redraw below triggers via `eglSwapBuffers`. The EGL surface
        // itself is brought to the new physical size by
        // `apply_pending_resize` before that paint.
        surface.set_buffer_scale(self.buffer_scale());
        self.needs_redraw = true;
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wayland_client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
        // The compositor presented our last commit and is ready for the
        // next frame. Only clear the gate — deliberately do NOT set
        // `needs_redraw` here, or every paint (which requests the next
        // callback) would schedule another paint forever, pinning the
        // GUI at the refresh rate even when idle. Invalidation comes
        // from input, commands, configure, and egui's repaint requests.
        self.frame_callback_pending = None;
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(kb) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(kb);
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(p) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(p);
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.input
            .process_key(&event, true, &mut self.pending_events);
        self.needs_redraw = true;
    }
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.input
            .process_key(&event, true, &mut self.pending_events);
        self.needs_redraw = true;
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.input
            .process_key(&event, false, &mut self.pending_events);
        self.needs_redraw = true;
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        modifiers: SctkModifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
        self.input.set_modifiers(modifiers);
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            self.input.process_pointer(event, &mut self.pending_events);
        }
        self.needs_redraw = true;
    }
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.close_requested = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let (w, h) = (
            configure
                .new_size
                .0
                .map(NonZeroU32::get)
                .unwrap_or(self.size.0),
            configure
                .new_size
                .1
                .map(NonZeroU32::get)
                .unwrap_or(self.size.1),
        );
        self.pending_size = Some((w, h));
        if !self.configured {
            self.visible = true;
        }
        self.configured = true;
        self.needs_redraw = true;
        // Store the negotiated decoration mode. We request server-side
        // decorations, but the compositor may force client-side (or never
        // offer SSD), in which case `paint` draws the CSD fallback frame so
        // the window always has a border + titlebar + close button.
        //
        // `WPG_FORCE_CSD` (any non-empty value) forces `Client` mode
        // regardless of what the compositor negotiated. This exists purely so
        // the CSD fallback frame can be rendered/verified on an SSD compositor
        // (Hyprland/Sway/KWin) without needing a GNOME/Mutter session; it has
        // no effect on the production default.
        self.decoration_mode = if std::env::var_os("WPG_FORCE_CSD").is_some() {
            DecorationMode::Client
        } else {
            configure.decoration_mode
        };
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

// SCTK delegate wiring
delegate_compositor!(State);
delegate_output!(State);
delegate_seat!(State);
delegate_keyboard!(State);
delegate_pointer!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_registry!(State);
