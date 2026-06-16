//! Per-frame paint / redraw logic and EGL surface management.

use std::time::{Duration, Instant};

use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::QueueHandle;

use crate::app::EditorApp;
use crate::egl_context::EglContext;
use crate::error::EditorError;

use super::debug::dump_ppm;
use super::decorations::{FrameLayout, BORDER_WIDTH, CLOSE_BUTTON_SIZE};
use super::state::State;

/// A [`FrameRect`](super::decorations::FrameRect) as an [`egui::Rect`].
fn to_egui_rect(r: super::decorations::FrameRect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(r.min_x, r.min_y),
        egui::pos2(r.max_x, r.max_y),
    )
}

/// Draw the CSD fallback chrome (border + titlebar + close button) onto `ui`
/// and run the app UI inside the inset content rect. Returns `true` if the
/// close button was clicked this frame, so the caller can request a close.
///
/// `ui` is the root UI covering the whole surface (`screen_rect`). The chrome
/// is intentionally minimal: a one-pixel border, a flat titlebar strip with the
/// window title, and a square close button at the right end. The geometry
/// mirrors [`FrameLayout`] exactly so the live hit area and the unit-tested
/// [`FrameLayout::is_close_click`] agree.
fn draw_csd_frame(
    ui: &mut egui::Ui,
    title: &str,
    layout: FrameLayout,
    app: &mut dyn EditorApp,
) -> bool {
    use egui::{Align2, Color32, FontId, Sense, Stroke, StrokeKind, UiBuilder, Vec2};

    // Quiet neutral chrome that reads on any palette without importing one.
    const FRAME_BG: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x23);
    const BORDER: Color32 = Color32::from_rgb(0x3a, 0x3d, 0x45);
    const TITLE_TEXT: Color32 = Color32::from_rgb(0xc8, 0xc8, 0xce);
    const CLOSE_HOVER: Color32 = Color32::from_rgb(0xc0, 0x3a, 0x3a);
    const CLOSE_GLYPH: Color32 = Color32::from_rgb(0xe0, 0xe0, 0xe0);

    let full = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(layout.width, layout.height));
    let titlebar = to_egui_rect(layout.titlebar_rect());
    let close_rect = to_egui_rect(layout.close_button_rect());

    let painter = ui.painter().clone();
    // Outer border around the whole window.
    painter.rect_stroke(full, 0.0, Stroke::new(BORDER_WIDTH, BORDER), StrokeKind::Inside);
    // Titlebar background + bottom separator.
    painter.rect_filled(titlebar, 0.0, FRAME_BG);
    painter.line_segment(
        [titlebar.left_bottom(), titlebar.right_bottom()],
        Stroke::new(BORDER_WIDTH, BORDER),
    );
    // Title text, left-aligned with a small inset.
    painter.text(
        egui::pos2(titlebar.min.x + 10.0, titlebar.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.0),
        TITLE_TEXT,
    );

    // Close button. Use an interactive region so hover/click reuse the
    // existing pointer event pipeline (no manual hit-testing here); the rect
    // is identical to `FrameLayout::close_button_rect`, the same geometry the
    // unit test drives.
    let resp = ui.interact(close_rect, egui::Id::new("wpg_csd_close"), Sense::click());
    if resp.hovered() {
        painter.rect_filled(close_rect, 0.0, CLOSE_HOVER);
    }
    // Draw an "x" glyph.
    let c = close_rect.center();
    let r = CLOSE_BUTTON_SIZE * 0.22;
    painter.line_segment(
        [c - Vec2::splat(r), c + Vec2::splat(r)],
        Stroke::new(1.5, CLOSE_GLYPH),
    );
    painter.line_segment(
        [c + Vec2::new(r, -r), c + Vec2::new(-r, r)],
        Stroke::new(1.5, CLOSE_GLYPH),
    );
    let close_clicked = resp.clicked();

    // Run the app UI inside the inset content rect.
    let content = to_egui_rect(layout.content_rect());
    ui.scope_builder(UiBuilder::new().max_rect(content), |content_ui| {
        content_ui.set_clip_rect(content);
        app.ui(content_ui);
    });

    close_clicked
}

/// Apply any pending resize (from a configure or `Command::Resize`) to the
/// EGL surface. Sets `state.needs_redraw` if the size actually changed.
///
/// The EGL surface is unconditionally brought to `state.physical_size()`
/// (`EglContext::resize` is a no-op when unchanged), so a scale change —
/// which alters the physical size without touching `pending_size` — also
/// resizes the buffer before the next paint.
pub(super) fn apply_pending_resize(state: &mut State, egl_ctx: &mut EglContext) {
    if let Some((w, h)) = state.pending_size.take() {
        if (w, h) != state.size {
            state.size = (w, h);
            state.needs_redraw = true;
        }
    }
    let (pw, ph) = state.physical_size();
    egl_ctx.resize(pw, ph);
}

/// Build one egui frame, paint it via `egui_glow`, and swap buffers.
///
/// Returns `Ok(())` on success. `state.needs_redraw` is set to `true` again if
/// egui requests another repaint within 50 ms.
pub(super) fn paint_frame(
    state: &mut State,
    app: &mut dyn EditorApp,
    egl_ctx: &mut EglContext,
    gl: &std::sync::Arc<glow::Context>,
    painter: &mut egui_glow::Painter,
    start_time: Instant,
    qh: &QueueHandle<State>,
) -> Result<(), EditorError> {
    state.needs_redraw = false;

    egl_ctx.make_current()?;

    // Build the egui frame. `pixels_per_point` is the same integer buffer
    // scale the EGL surface and `set_buffer_scale` use (see
    // `State::buffer_scale`), so the GL viewport below always matches the
    // buffer exactly.
    let (w, h) = state.size;
    let pixels_per_point = state.buffer_scale() as f32;

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

    // In CSD mode the compositor gives us no chrome, so we draw our own frame
    // (border + titlebar + close button) and run the app UI in the inset rect.
    // The close button feeds the same `close_requested` path as the SSD
    // `xdg_toplevel.close` event (drained in the event loop).
    let csd = state.needs_csd();
    let title = state.title.clone();
    let layout = FrameLayout::new(w as f32, h as f32);
    let mut csd_close = false;
    let full_output = state.egui_ctx.run_ui(raw_input, |ui| {
        if csd {
            if draw_csd_frame(ui, &title, layout, app) {
                csd_close = true;
            }
        } else {
            app.ui(ui);
        }
    });
    if csd_close {
        state.close_requested = true;
        // Keep redrawing so the close is processed promptly even if the app
        // would otherwise idle.
        state.needs_redraw = true;
    }

    let clipped_primitives = state
        .egui_ctx
        .tessellate(full_output.shapes, pixels_per_point);

    // Clear and paint. Same physical size the EGL surface was resized to
    // in `apply_pending_resize` — one rounding rule, no drift.
    use glow::HasContext;
    let (pw, ph) = state.physical_size();
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
    // debugging. Set WPG_DUMP_FRAME=/tmp/wpg.ppm to capture a frame to a PPM.
    // By default the *first* painted frame is captured; set WPG_DUMP_FRAME_AT
    // to a frame number (1-based) to capture a later, fully-settled frame
    // instead (the first frame can predate the font atlas / app layout).
    if let Ok(path) = std::env::var("WPG_DUMP_FRAME") {
        static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let target = std::env::var("WPG_DUMP_FRAME_AT")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1)
            .max(1);
        let n = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if n >= target && !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
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

    // Ask the compositor to tell us when it wants the next frame for
    // this surface. Requested *before* the swap because `eglSwapBuffers`
    // is what commits the surface, and a frame request only rides on the
    // commit that follows it. SCTK's `delegate_compositor!` routes the
    // callback (userdata = the surface) to `CompositorHandler::frame`,
    // which clears this gate. The swap itself never blocks: the event
    // loop set eglSwapInterval(0), so pacing happens here, visibly to
    // input handling, instead of inside the GL driver.
    let surface = state.window.wl_surface();
    surface.frame(qh, surface.clone());
    state.frame_callback_pending = Some(Instant::now());

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

    Ok(())
}
