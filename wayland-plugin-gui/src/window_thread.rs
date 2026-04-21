//! The editor thread.
//!
//! Owns a Wayland connection, an xdg_toplevel window, an EGL context and an
//! `egui_glow::Painter`. Runs an SCTK calloop event loop, consumes commands
//! from the public [`Editor`] handle, and repaints on demand.

use std::num::NonZeroU32;
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel as calloop_channel;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers as SctkModifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_xdg_shell, delegate_xdg_window, registry_handlers,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use crate::app::EditorApp;
use crate::editor::EditorOptions;
use crate::egl_context::EglContext;
use crate::error::EditorError;
use crate::input::InputState;

/// Commands the public `Editor` handle can send to the editor thread.
pub enum Command {
    Show,
    Hide,
    Resize(u32, u32),
    Repaint,
    Quit,
}

pub struct EditorThread;

impl EditorThread {
    pub fn run(
        app: Box<dyn EditorApp>,
        options: EditorOptions,
        cmd_channel: calloop_channel::Channel<Command>,
        ready_tx: SyncSender<Result<(), EditorError>>,
    ) {
        let result = Self::run_inner(app, options, cmd_channel, ready_tx.clone());
        if let Err(err) = result {
            eprintln!("wpg: editor thread exited with error: {}", err);
            // Try to send the error; the main thread may have given up already.
            let _ = ready_tx.try_send(Err(err));
        }
    }

    fn run_inner(
        mut app: Box<dyn EditorApp>,
        options: EditorOptions,
        cmd_channel: calloop_channel::Channel<Command>,
        ready_tx: SyncSender<Result<(), EditorError>>,
    ) -> Result<(), EditorError> {
        // -------- Wayland connection + globals --------
        let conn =
            Connection::connect_to_env().map_err(|e| EditorError::WaylandConnect(e.to_string()))?;

        let (globals, event_queue) = registry_queue_init::<State>(&conn)
            .map_err(|e| EditorError::WaylandConnect(e.to_string()))?;
        let qh: QueueHandle<State> = event_queue.handle();

        let compositor = CompositorState::bind(&globals, &qh)
            .map_err(|_| EditorError::GlobalsMissing("wl_compositor"))?;
        let xdg_shell = XdgShell::bind(&globals, &qh)
            .map_err(|_| EditorError::GlobalsMissing("xdg_wm_base"))?;

        let surface = compositor.create_surface(&qh);
        let window = xdg_shell.create_window(surface, WindowDecorations::ServerDefault, &qh);
        window.set_title(&options.title);
        window.set_app_id("com.resonance.wavetable");
        window.set_min_size(Some(options.min_size));
        window.commit();

        // Create the calloop event loop.
        let mut event_loop: EventLoop<State> =
            EventLoop::try_new().map_err(|e| EditorError::WaylandConnect(e.to_string()))?;
        let loop_handle = event_loop.handle();

        // Insert the command channel source.
        loop_handle
            .insert_source(cmd_channel, |event, _, state| {
                if let calloop_channel::Event::Msg(cmd) = event {
                    match cmd {
                        Command::Show => {
                            state.visible = true;
                            state.needs_redraw = true;
                        }
                        Command::Hide => {
                            state.visible = false;
                            // On Wayland there's no explicit "hide window" — we
                            // stop drawing. Unmap would need a null buffer commit.
                        }
                        Command::Resize(w, h) => {
                            state.pending_size = Some((w, h));
                            state.needs_redraw = true;
                        }
                        Command::Repaint => {
                            state.needs_redraw = true;
                        }
                        Command::Quit => {
                            state.running = false;
                        }
                    }
                }
            })
            .map_err(|e| EditorError::WaylandConnect(format!("calloop channel: {e}")))?;

        // Hook Wayland events into calloop.
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle.clone())
            .map_err(|e| EditorError::WaylandConnect(format!("wayland source: {e}")))?;

        let mut state = State {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            window,
            conn: conn.clone(),
            keyboard: None,
            pointer: None,
            size: options.initial_size,
            pending_size: None,
            scale: 1.0,
            visible: false,
            running: true,
            configured: false,
            needs_redraw: true,
            close_requested: false,
            input: InputState::new(),
            pending_events: Vec::new(),
            egui_ctx: egui::Context::default(),
        };

        // Drive the loop until we get our first configure event, so EGL can
        // set up with the real size.
        while !state.configured && state.running {
            if let Err(e) = event_loop.dispatch(Duration::from_millis(16), &mut state) {
                return Err(EditorError::WaylandConnect(format!("dispatch: {e}")));
            }
        }

        if !state.running {
            let _ = ready_tx.try_send(Err(EditorError::WaylandConnect(
                "quit before configure".to_string(),
            )));
            return Ok(());
        }

        // -------- EGL + glow + egui painter --------
        let mut egl_ctx = EglContext::new(
            &state.conn,
            state.window.wl_surface(),
            state.size,
            state.scale as i32,
        )?;
        egl_ctx.make_current()?;

        let gl = unsafe { glow::Context::from_loader_function(|s| egl_ctx.get_proc_address(s)) };
        let gl = std::sync::Arc::new(gl);

        let mut painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|e| EditorError::GlLoad(e.to_string()))?;

        // Ready to go — signal the caller that init succeeded.
        let _ = ready_tx.try_send(Ok(()));

        let start_time = Instant::now();

        // -------- Main loop --------
        let frame_budget = Duration::from_millis(16);
        while state.running {
            if let Err(e) = event_loop.dispatch(frame_budget, &mut state) {
                eprintln!("wayland-plugin-gui: dispatch error: {e}");
                break;
            }

            if state.close_requested {
                app.on_close();
                state.running = false;
                continue;
            }

            // Apply any pending resize from a configure or Command::Resize.
            if let Some((w, h)) = state.pending_size.take() {
                if (w, h) != state.size {
                    state.size = (w, h);
                    let s = state.scale.max(1.0) as i32;
                    egl_ctx.resize(w as i32 * s, h as i32 * s);
                    state.needs_redraw = true;
                }
            }

            if state.visible && state.needs_redraw {
                state.needs_redraw = false;

                egl_ctx.make_current()?;

                // Build the egui frame.
                let (w, h) = state.size;
                let pixels_per_point = state.scale;

                // Provide pixels_per_point via the viewport info, not via
                // Context::set_pixels_per_point — the latter only takes effect
                // on the *next* pass, so the font atlas would be sized for the
                // wrong pp on the first frame, causing text UVs to reference
                // empty atlas regions.
                let mut viewports = egui::ViewportIdMap::default();
                viewports.insert(
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        native_pixels_per_point: Some(pixels_per_point),
                        ..Default::default()
                    },
                );
                let raw_input = egui::RawInput {
                    viewport_id: egui::ViewportId::ROOT,
                    viewports,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(w as f32, h as f32),
                    )),
                    events: std::mem::take(&mut state.pending_events),
                    modifiers: state.input.modifiers(),
                    time: Some(start_time.elapsed().as_secs_f64()),
                    focused: true,
                    ..Default::default()
                };

                let full_output = state.egui_ctx.run_ui(raw_input, |ui| app.ui(ui));

                let clipped_primitives = state
                    .egui_ctx
                    .tessellate(full_output.shapes, pixels_per_point);

                // Clear and paint.
                use glow::HasContext;
                let pw = (w as f32 * pixels_per_point) as i32;
                let ph = (h as f32 * pixels_per_point) as i32;
                unsafe {
                    gl.viewport(0, 0, pw, ph);
                    gl.clear_color(0.08, 0.08, 0.10, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                }

                painter.paint_and_update_textures(
                    [pw as u32, ph as u32],
                    pixels_per_point,
                    &clipped_primitives,
                    &full_output.textures_delta,
                );

                // Optional framebuffer dump via env var — kept for dev
                // debugging. Set WPG_DUMP_FRAME=/tmp/wpg.ppm to capture the
                // next frame.
                if let Ok(path) = std::env::var("WPG_DUMP_FRAME") {
                    static DONE: std::sync::atomic::AtomicBool =
                        std::sync::atomic::AtomicBool::new(false);
                    if !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        let mut buf = vec![0u8; (pw * ph * 4) as usize];
                        unsafe {
                            gl.read_pixels(
                                0,
                                0,
                                pw,
                                ph,
                                glow::RGBA,
                                glow::UNSIGNED_BYTE,
                                glow::PixelPackData::Slice(Some(&mut buf)),
                            );
                        }
                        let _ = dump_ppm(&path, pw as u32, ph as u32, &buf);
                    }
                }

                egl_ctx.swap_buffers()?;

                // If egui wants a repaint soon, schedule one.
                let repaint_after = full_output
                    .viewport_output
                    .values()
                    .map(|v| v.repaint_delay)
                    .min()
                    .unwrap_or(Duration::from_millis(16));
                if repaint_after < Duration::from_millis(50) {
                    state.needs_redraw = true;
                }
            }
        }

        painter.destroy();
        drop(egl_ctx);
        drop(gl);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// State: holds everything SCTK dispatch handlers mutate.
// ---------------------------------------------------------------------------

struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    window: Window,
    conn: Connection,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    size: (u32, u32),
    pending_size: Option<(u32, u32)>,
    scale: f32,
    visible: bool,
    running: bool,
    configured: bool,
    needs_redraw: bool,
    close_requested: bool,
    input: InputState,
    pending_events: Vec<egui::Event>,
    egui_ctx: egui::Context,
}

// --- SCTK delegate impls ---------------------------------------------------

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        new_factor: i32,
    ) {
        self.scale = new_factor as f32;
        self.input.set_scale(self.scale);
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
        self.needs_redraw = true;
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
        // Decoration mode is negotiated automatically (server-side preferred);
        // both modes are acceptable.
        let _ = configure.decoration_mode;
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

#[allow(dead_code)]
fn dump_ppm(path: &str, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    write!(f, "P6\n{} {}\n255\n", w, h)?;
    for y in (0..h).rev() {
        let row = (y * w * 4) as usize;
        for x in 0..w {
            let i = row + (x as usize * 4);
            f.write_all(&rgba[i..i + 3])?;
        }
    }
    Ok(())
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
