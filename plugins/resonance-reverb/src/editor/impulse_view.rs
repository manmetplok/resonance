//! Hero "impulse tail" visualisation.
//!
//! A layered composite showing the reverb's impulse response at a
//! glance. Layers, back to front:
//!
//!   1. grid + dB/time guides
//!   2. live wet-RMS trace polygon (darker, sits *behind* the analytic
//!      envelope so real audio visibly "fills" the theoretical shape)
//!   3. pre-delay pulse marker
//!   4. 12 early-reflection spike lollipops (L above axis, R below)
//!   5. analytic RT60 decay polygon + stroked outline, with a mild
//!      sinusoidal modulation ripple and a freeze override

use wayland_plugin_gui::egui;

use crate::viz::TAIL_HISTORY_LEN;

use super::theme;
use super::ReverbEditorApp;

/// Total horizontal span of the plot in milliseconds. Chosen so short
/// rooms and long cathedrals both read nicely without rescaling.
const WINDOW_MS: f32 = 3000.0;

const FLOOR_DB: f32 = -60.0;
const CEILING_DB: f32 = 0.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, app: &ReverbEditorApp) {
    draw_background(painter, rect);

    let inner = rect.shrink(10.0);
    if inner.width() <= 20.0 || inner.height() <= 20.0 {
        return;
    }

    // Horizontal axis sits at the vertical centre — left side shows the
    // decay envelope + L-channel ER spikes above; the R-channel ER spikes
    // hang below so the plot reads as a stereo lollipop.
    let axis_y = inner.top() + inner.height() * 0.60;
    let upper = egui::Rect::from_min_max(inner.min, egui::pos2(inner.right(), axis_y));
    let lower = egui::Rect::from_min_max(egui::pos2(inner.left(), axis_y), inner.max);

    draw_grid(painter, inner, axis_y);

    // Pull param snapshot once per frame.
    let predelay_ms = app.params.predelay.value();
    let decay_s = app.params.decay.value();
    let mod_rate = app.params.mod_rate.value();
    let mod_depth = app.params.mod_depth.value();
    let er_level = app.params.er_level.value();
    let frozen = app.params.freeze.value();

    // 2. Live wet-RMS trace, drawn behind the analytic envelope.
    draw_live_trace(painter, inner, axis_y, app);

    // 3. Pre-delay pulse marker.
    let predelay_x = ms_to_x(predelay_ms, inner);
    if predelay_ms > 0.1 {
        painter.line_segment(
            [
                egui::pos2(predelay_x, inner.top() + 2.0),
                egui::pos2(predelay_x, inner.bottom() - 2.0),
            ],
            egui::Stroke::new(1.5, theme::WARN),
        );
        painter.text(
            egui::pos2(predelay_x + 4.0, inner.top() + 8.0),
            egui::Align2::LEFT_TOP,
            format!("pre {:.0}ms", predelay_ms),
            egui::FontId::proportional(9.0),
            theme::WARN,
        );
    }

    // 4. Early-reflection spikes.
    draw_er_spikes(painter, inner, axis_y, app, predelay_ms, er_level);

    // 5. Analytic decay polygon + outline.
    draw_decay_envelope(
        painter,
        upper,
        lower,
        axis_y,
        predelay_ms,
        decay_s,
        mod_rate,
        mod_depth,
        frozen,
    );

    // Axis line (drawn last so it sits on top of the trace fill edge).
    painter.line_segment(
        [
            egui::pos2(inner.left(), axis_y),
            egui::pos2(inner.right(), axis_y),
        ],
        egui::Stroke::new(0.8, theme::BORDER),
    );

    // Header label top-left.
    painter.text(
        egui::pos2(inner.left(), rect.top() + 6.0),
        egui::Align2::LEFT_TOP,
        "IMPULSE TAIL",
        egui::FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
}

fn draw_background(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 3.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect, axis_y: f32) {
    // Vertical time markers every 500 ms up to WINDOW_MS.
    let mut t = 0.0;
    while t <= WINDOW_MS {
        let x = ms_to_x(t, rect);
        painter.line_segment(
            [
                egui::pos2(x, rect.top() + 2.0),
                egui::pos2(x, rect.bottom() - 2.0),
            ],
            egui::Stroke::new(0.4, theme::BORDER),
        );
        // Time label along the axis.
        let label = if t >= 1000.0 {
            format!("{:.1}s", t / 1000.0)
        } else {
            format!("{:.0}ms", t)
        };
        painter.text(
            egui::pos2(x + 3.0, axis_y + 2.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(9.0),
            theme::TEXT_DIM,
        );
        t += 500.0;
    }

    // Horizontal dB guides above the axis at -6, -12, -24 dB.
    for &db in &[-6.0, -12.0, -24.0] {
        let y = db_to_y(db, rect.top(), axis_y);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.3, theme::BORDER),
        );
        painter.text(
            egui::pos2(rect.right() - 4.0, y - 2.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{db:.0}"),
            egui::FontId::monospace(9.0),
            theme::TEXT_DIM,
        );
    }
}

fn draw_er_spikes(
    painter: &egui::Painter,
    rect: egui::Rect,
    axis_y: f32,
    app: &ReverbEditorApp,
    predelay_ms: f32,
    er_level: f32,
) {
    let (tap_times, tap_gains) = app.viz.read_er_taps();
    let upper_h = axis_y - rect.top();
    let lower_h = rect.bottom() - axis_y;
    let max_spike_up = upper_h * 0.85;
    let max_spike_dn = lower_h * 0.85;

    for i in 0..tap_times.len() {
        let (ms_l, ms_r) = tap_times[i];
        let (g_l, g_r) = tap_gains[i];

        let x_l = ms_to_x(predelay_ms + ms_l, rect);
        let x_r = ms_to_x(predelay_ms + ms_r, rect);
        // Skip taps outside the visible window.
        if x_l >= rect.left() && x_l <= rect.right() {
            let h = (g_l * er_level).clamp(0.0, 1.0) * max_spike_up;
            let stem_top = egui::pos2(x_l, axis_y - h);
            let stem_bot = egui::pos2(x_l, axis_y);
            painter.line_segment(
                [stem_top, stem_bot],
                egui::Stroke::new(1.5, theme::ER_SPIKE),
            );
            painter.circle_filled(stem_top, 1.8, theme::ER_SPIKE);
        }
        if x_r >= rect.left() && x_r <= rect.right() {
            let h = (g_r * er_level).clamp(0.0, 1.0) * max_spike_dn;
            let stem_top = egui::pos2(x_r, axis_y);
            let stem_bot = egui::pos2(x_r, axis_y + h);
            painter.line_segment(
                [stem_top, stem_bot],
                egui::Stroke::new(1.5, theme::ER_SPIKE),
            );
            painter.circle_filled(stem_bot, 1.8, theme::ER_SPIKE);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_decay_envelope(
    painter: &egui::Painter,
    upper: egui::Rect,
    _lower: egui::Rect,
    axis_y: f32,
    predelay_ms: f32,
    decay_s: f32,
    mod_rate: f32,
    mod_depth: f32,
    frozen: bool,
) {
    // Build the curve as a series of (x, y) points across the plot.
    let samples = 256usize;
    let rt60 = decay_s.max(0.05);
    let start_x = ms_to_x(predelay_ms, upper);
    let end_x = upper.right();
    if end_x <= start_x + 2.0 {
        return;
    }

    let mut top_pts: Vec<egui::Pos2> = Vec::with_capacity(samples + 2);
    // Anchor the polygon on the axis at the predelay mark.
    top_pts.push(egui::pos2(start_x, axis_y));

    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f32())
        .unwrap_or(0.0);

    for i in 0..samples {
        let t = i as f32 / (samples - 1) as f32;
        let x = start_x + t * (end_x - start_x);
        let t_ms = x_to_ms(x, upper) - predelay_ms;
        if t_ms < 0.0 {
            continue;
        }
        let t_s = t_ms * 0.001;
        let db = if frozen {
            // Flat plateau near the peak.
            -3.0
        } else {
            // RT60 = time for -60 dB.
            -60.0 * t_s / rt60
        };

        // Modulation ripple: small wobble in dB space, animated in wall-clock
        // time so it's visually alive even when the audio thread is idle.
        let ripple = if mod_rate > 0.01 {
            (time * std::f32::consts::TAU * mod_rate + t * 4.0).sin() * mod_depth * 2.5
        } else {
            0.0
        };

        let y = db_to_y(db + ripple, upper.top(), axis_y);
        top_pts.push(egui::pos2(x, y));
    }
    // Close the polygon down to the axis on the right edge.
    top_pts.push(egui::pos2(end_x, axis_y));

    // Filled polygon as a triangle fan rooted at the axis anchor.
    // Using explicit triangles avoids ear-clipping issues for a strip
    // that can dip below/above on successive samples.
    let fill = theme::TAIL_GLOW;
    let mut mesh = egui::epaint::Mesh::default();
    if top_pts.len() >= 3 {
        for pair in top_pts.windows(2).skip(1) {
            let p0 = pair[0];
            let p1 = pair[1];
            let b = mesh.vertices.len() as u32;
            mesh.colored_vertex(egui::pos2(p0.x, axis_y), fill);
            mesh.colored_vertex(p0, fill);
            mesh.colored_vertex(p1, fill);
            mesh.colored_vertex(egui::pos2(p1.x, axis_y), fill);
            mesh.add_triangle(b, b + 1, b + 2);
            mesh.add_triangle(b, b + 2, b + 3);
        }
    }
    painter.add(egui::Shape::mesh(mesh));

    // Crisp outline on top.
    let outline: Vec<egui::Pos2> = top_pts.to_vec();
    painter.add(egui::Shape::line(
        outline,
        egui::Stroke::new(1.4, theme::ACCENT),
    ));
}

fn draw_live_trace(painter: &egui::Painter, rect: egui::Rect, axis_y: f32, app: &ReverbEditorApp) {
    // Snapshot the ring buffer.
    let samples: [f32; TAIL_HISTORY_LEN] = {
        let guard = app.viz.tail.lock();
        let mut out = [0.0f32; TAIL_HISTORY_LEN];
        for (i, v) in guard.iter_chrono().enumerate() {
            out[i] = v;
        }
        out
    };

    // Find the peak for auto-scaling, fall back to a gentle floor so a
    // silent block doesn't erase the envelope.
    let peak = samples.iter().copied().fold(0.0f32, f32::max).max(1e-4);

    // Map samples to the same horizontal window as the analytic curve,
    // anchored at the left edge. Newest samples on the right.
    let top_y = axis_y - (axis_y - rect.top()) * 0.85;

    let mut pts = Vec::with_capacity(TAIL_HISTORY_LEN + 2);
    pts.push(egui::pos2(rect.left(), axis_y));
    for (i, &v) in samples.iter().enumerate() {
        let t = i as f32 / (TAIL_HISTORY_LEN - 1) as f32;
        let x = rect.left() + t * rect.width();
        let norm = (v / peak).clamp(0.0, 1.0).powf(0.6);
        let y = axis_y + (top_y - axis_y) * norm;
        pts.push(egui::pos2(x, y));
    }
    pts.push(egui::pos2(rect.right(), axis_y));

    // Dim fill sitting behind the analytic envelope.
    let fill = egui::Color32::from_rgba_premultiplied(0x0f, 0x22, 0x30, 0x60);
    let mut mesh = egui::epaint::Mesh::default();
    for pair in pts.windows(2).skip(1) {
        let p0 = pair[0];
        let p1 = pair[1];
        let b = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(p0.x, axis_y), fill);
        mesh.colored_vertex(p0, fill);
        mesh.colored_vertex(p1, fill);
        mesh.colored_vertex(egui::pos2(p1.x, axis_y), fill);
        mesh.add_triangle(b, b + 1, b + 2);
        mesh.add_triangle(b, b + 2, b + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
}

// -- Coordinate helpers ---------------------------------------------------

fn ms_to_x(ms: f32, rect: egui::Rect) -> f32 {
    let t = (ms / WINDOW_MS).clamp(0.0, 1.0);
    rect.left() + t * rect.width()
}

fn x_to_ms(x: f32, rect: egui::Rect) -> f32 {
    let t = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    t * WINDOW_MS
}

/// Convert a dB value (CEILING_DB..FLOOR_DB) to a Y coordinate in the
/// region between `top` and `axis_y` — CEILING_DB sits at `top`, FLOOR_DB
/// at `axis_y`.
fn db_to_y(db: f32, top: f32, axis_y: f32) -> f32 {
    let clamped = db.clamp(FLOOR_DB, CEILING_DB);
    let t = (CEILING_DB - clamped) / (CEILING_DB - FLOOR_DB);
    top + t * (axis_y - top)
}
