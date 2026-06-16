//! Editor thread entry point and main calloop event loop.

use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::reexports::calloop::channel as calloop_channel;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::xdg::window::WindowDecorations;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::globals::registry_queue_init;
use wayland_client::{Connection, QueueHandle};

use crate::app::EditorApp;
use crate::editor::EditorOptions;
use crate::egl_context::EglContext;
use crate::error::EditorError;
use crate::input::InputState;

use super::paint::{apply_pending_resize, paint_frame};
use super::state::State;

/// Commands the public `Editor` handle can send to the editor thread.
pub enum Command {
    Show,
    Hide,
    Resize(u32, u32),
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
        // Resolve the decoration policy once. The default is "prefer client":
        // the runtime draws its own frame (border + titlebar + close button)
        // on every compositor. This is deliberate — "server-side decorations"
        // does NOT imply a close button: wlroots compositors (Hyprland, Sway)
        // honour an SSD request but render only a thin border with no titlebar
        // and no close affordance, so a window relying on SSD there cannot be
        // closed from the window itself (the original todo #216 report). Always
        // drawing our own frame guarantees an identical, working close button
        // everywhere. `WPG_FORCE_SSD` opts back into the #215 SSD-negotiation
        // behaviour (request server-side, draw CSD only when the compositor
        // forces client) for hosts/tests that want the native titlebar where
        // one actually exists (e.g. KWin).
        let prefer_server = std::env::var_os("WPG_FORCE_SSD").is_some();
        let (requested, initial_mode) = if prefer_server {
            // Ask for SSD; the configure handler reads back the negotiated mode
            // and only draws CSD when the compositor forces client-side.
            (
                WindowDecorations::RequestServer,
                smithay_client_toolkit::shell::xdg::window::DecorationMode::Server,
            )
        } else {
            // Ask the compositor NOT to decorate; we draw the frame ourselves
            // so there is always a working close button. Seed the mode to
            // `Client` so `needs_csd()` is true from the very first paint, even
            // before the first configure arrives.
            (
                WindowDecorations::RequestClient,
                smithay_client_toolkit::shell::xdg::window::DecorationMode::Client,
            )
        };
        let window = xdg_shell.create_window(surface, requested, &qh);
        window.set_title(&options.title);
        window.set_app_id(&options.app_id);
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
            frame_callback_pending: None,
            close_requested: false,
            input: InputState::new(),
            pending_events: Vec::new(),
            egui_ctx: egui::Context::default(),
            // Seed from the resolved policy: `Client` (draw our own frame) by
            // default, `Server` when `WPG_FORCE_SSD` opts into SSD negotiation.
            // The configure handler updates this per the policy below.
            decoration_mode: initial_mode,
            prefer_server,
            title: options.title.clone(),
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
            state.buffer_scale(),
        )?;
        egl_ctx.make_current()?;
        // Swaps must not block: frame pacing is done explicitly below via
        // wl_surface.frame() callbacks (see the main loop), not inside the
        // GL driver where it would stall input handling too.
        egl_ctx.set_swap_interval_zero()?;

        let gl = unsafe { glow::Context::from_loader_function(|s| egl_ctx.get_proc_address(s)) };
        let gl = std::sync::Arc::new(gl);

        let mut painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|e| EditorError::GlLoad(e.to_string()))?;

        // Ready to go — signal the caller that init succeeded.
        let _ = ready_tx.try_send(Ok(()));

        let start_time = Instant::now();

        // -------- Main loop --------
        //
        // Frame pacing is compositor-driven: every paint requests a
        // wl_surface.frame() callback (see `paint_frame`) and the next
        // paint waits for it. The compositor thus sets the cadence (the
        // monitor's refresh rate for a visible surface, nothing at all
        // for an occluded one) instead of a fixed 16 ms tick. The
        // dispatch timeout is only a parking budget — any Wayland event
        // or editor command wakes calloop immediately via its fds, so
        // input latency never depends on these numbers.

        // A frame callback this overdue is treated as lost (compositor
        // restart, unmap race, callback-withholding compositor) and the
        // gate is forced open, so a stalled compositor degrades the GUI
        // to ~4 fps instead of freezing it.
        const FRAME_CALLBACK_STALL: Duration = Duration::from_millis(250);
        // Parking budget when there's nothing to paint.
        const IDLE_BUDGET: Duration = Duration::from_millis(500);

        while state.running {
            let timeout = if state.visible && state.needs_redraw {
                match state.frame_callback_pending {
                    // Waiting on the compositor: park until the callback
                    // arrives (wakes dispatch) or the stall deadline.
                    Some(since) => FRAME_CALLBACK_STALL
                        .saturating_sub(since.elapsed())
                        .max(Duration::from_millis(1)),
                    // Gate open: poll without parking and paint below.
                    None => Duration::ZERO,
                }
            } else {
                IDLE_BUDGET
            };
            if let Err(e) = event_loop.dispatch(timeout, &mut state) {
                eprintln!("wayland-plugin-gui: dispatch error: {e}");
                break;
            }

            if state.close_requested {
                app.on_close();
                state.running = false;
                continue;
            }

            // Apply any pending resize from a configure or Command::Resize.
            apply_pending_resize(&mut state, &mut egl_ctx);

            // Declare a long-overdue frame callback lost.
            if let Some(since) = state.frame_callback_pending {
                if since.elapsed() >= FRAME_CALLBACK_STALL {
                    state.frame_callback_pending = None;
                }
            }

            if state.visible && state.needs_redraw && state.frame_callback_pending.is_none() {
                paint_frame(
                    &mut state,
                    app.as_mut(),
                    &mut egl_ctx,
                    &gl,
                    &mut painter,
                    start_time,
                    &qh,
                )?;
            }
        }

        painter.destroy();
        drop(egl_ctx);
        drop(gl);
        Ok(())
    }
}
