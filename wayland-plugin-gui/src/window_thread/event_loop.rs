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
            apply_pending_resize(&mut state, &mut egl_ctx);

            if state.visible && state.needs_redraw {
                paint_frame(
                    &mut state,
                    app.as_mut(),
                    &mut egl_ctx,
                    &gl,
                    &mut painter,
                    start_time,
                )?;
            }
        }

        painter.destroy();
        drop(egl_ctx);
        drop(gl);
        Ok(())
    }
}
