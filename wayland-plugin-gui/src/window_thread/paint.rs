//! Per-frame paint / redraw logic and EGL surface management.

use std::time::{Duration, Instant};

use crate::app::EditorApp;
use crate::egl_context::EglContext;
use crate::error::EditorError;

use super::debug::dump_ppm;
use super::state::State;

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

    let full_output = state.egui_ctx.run_ui(raw_input, |ui| app.ui(ui));

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
    // debugging. Set WPG_DUMP_FRAME=/tmp/wpg.ppm to capture the
    // next frame.
    if let Ok(path) = std::env::var("WPG_DUMP_FRAME") {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
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

    Ok(())
}
