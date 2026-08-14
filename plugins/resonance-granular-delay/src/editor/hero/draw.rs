//! The static frame of the hero band: backdrop silhouette, division
//! ticks, pitch ruler, write head, delay tap, empty state and the
//! gesture legend. Painting only — it takes a [`HeroLayout`] and never
//! writes a parameter (ba todo #1265).

use egui::Ui;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::sync::{division_ms, DIVISION_LABELS};
use crate::viz::{GrainSnapshot, GranularViz, GRAIN_SLOTS, PEAK_BINS};

use super::super::theme;
use super::cloud::{draw_grain_cloud, draw_scale_lanes};
use super::layout::HeroLayout;

/// Vertical inset of the head/tap lines from the band edges, px.
const LINE_INSET: f32 = 14.0;

/// Draw the hero band into the available rect and return the frame's
/// layout, which the caller hands to [`super::interact`] — the gestures
/// grab by its `tap_hit`.
pub fn draw(ui: &mut Ui, params: &GranularDelayParams, viz: &GranularViz) -> HeroLayout {
    let canvas = ui.available_rect_before_wrap();
    let layout = HeroLayout::new(canvas, viz.read_delay_ms());
    let painter = ui.painter_at(canvas);
    let frozen = params.freeze.value();
    let now = ui.input(|i| i.time);

    painter.rect_filled(canvas, theme::RADIUS_PANEL, theme::BG_1);
    if frozen {
        // Amber wash over the held buffer (freeze state, req-5).
        painter.rect_filled(layout.plot, theme::RADIUS_PANEL, theme::WARM.gamma_multiply(0.035));
    }

    // Backdrop silhouette from the coarse buffer peaks (ba todo #1135).
    let mut peaks = [0.0f32; PEAK_BINS];
    let bin_ms = viz.read_peaks(&mut peaks);
    let peak_max = peaks.iter().fold(0.0f32, |m, &p| m.max(p));
    draw_backdrop(&painter, &layout, &peaks, bin_ms, frozen);

    if params.sync.value() {
        draw_division_ticks(&painter, &layout, params, viz.read_bpm());
    }
    draw_pitch_ruler(&painter, &layout);
    draw_scale_lanes(&painter, &layout, params);

    // Live grain cloud (ba todo #1143): decoded into a fixed
    // stack-local buffer — no per-frame heap allocation on this path.
    let mut grains = [GrainSnapshot::default(); GRAIN_SLOTS];
    let grain_count = viz.read_grains(&mut grains);
    draw_grain_cloud(
        &painter,
        &layout,
        &grains[..grain_count],
        params.texture.value(),
    );

    draw_write_head(&painter, &layout, frozen, now);
    draw_delay_tap(&painter, &layout, params, viz.read_delay_ms());

    // Empty state: no sounding grains and a silent buffer — flat line
    // only (already flat: silent peaks), plus the quiet hint. No fake
    // activity.
    let silent = viz.read_active_grains() == 0
        && viz.read_psola_voices() == 0
        && peak_max < 1.0e-4;
    if silent && !frozen {
        painter.text(
            egui::pos2(layout.plot.center().x, layout.mid_y() - 26.0),
            egui::Align2::CENTER_CENTER,
            "silence — the cloud appears when audio reaches the buffer",
            egui::FontId::proportional(11.0),
            theme::TEXT_4,
        );
    }

    // Gesture legend, bottom-left (req-2 affordance).
    painter.text(
        egui::pos2(layout.plot.left() + 12.0, layout.plot.bottom() - 18.0),
        egui::Align2::LEFT_CENTER,
        "drag tap ⇄ time · drag cloud ⇅ pitch · scroll = density",
        egui::FontId::proportional(9.0),
        theme::TEXT_4,
    );

    layout
}

/// Dim waveform/energy silhouette of the buffer contents: one vertical
/// span per ~3 px column, amplitude from the coarse peak bin at that
/// time offset. Silent buffers draw the flat midline.
fn draw_backdrop(
    painter: &egui::Painter,
    layout: &HeroLayout,
    peaks: &[f32; PEAK_BINS],
    bin_ms: f32,
    frozen: bool,
) {
    let color = if frozen {
        theme::WARM.gamma_multiply(0.16)
    } else {
        theme::TEXT_3.gamma_multiply(0.22)
    };
    let stroke = egui::Stroke::new(1.0, color);
    let mid = layout.mid_y();
    let amp_px = layout.canvas.height() * 0.09;

    // Flat midline under everything (also the whole empty state).
    painter.line_segment(
        [
            egui::pos2(layout.plot.left() + 8.0, mid),
            egui::pos2(layout.plot.right() - 2.0, mid),
        ],
        egui::Stroke::new(1.0, color.gamma_multiply(0.6)),
    );
    if bin_ms <= 0.0 {
        return;
    }

    let mut x = layout.plot.left() + 2.0;
    while x < layout.plot.right() - 2.0 {
        let ms = layout.ms_of_x(x);
        // Peaks are ordered oldest → newest with the head bin last.
        let back = (ms / bin_ms) as usize;
        if back < PEAK_BINS {
            let p = peaks[PEAK_BINS - 1 - back].clamp(0.0, 1.0);
            if p > 1.0e-4 {
                let a = (p.sqrt() * amp_px).max(0.5);
                painter.line_segment(
                    [egui::pos2(x, mid - a), egui::pos2(x, mid + a)],
                    stroke,
                );
            }
        }
        x += 3.0;
    }
}

/// Division tick marks across the buffer while tempo-synced (one per
/// division interval behind the write head), req-4.
fn draw_division_ticks(
    painter: &egui::Painter,
    layout: &HeroLayout,
    params: &GranularDelayParams,
    bpm: f32,
) {
    if bpm <= 0.0 {
        return;
    }
    let div_ms = division_ms(bpm, params.division.value() as usize);
    if div_ms <= 0.0 {
        return;
    }
    let stroke = egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.10));
    let top = layout.plot.top() + 26.0;
    let bottom = layout.plot.bottom() - 26.0;
    let window_ms = layout.window_seconds * 1000.0;
    let mut i = 1;
    while i as f32 * div_ms < window_ms {
        let x = layout.x_of_ms(i as f32 * div_ms);
        painter.line_segment([egui::pos2(x, top), egui::pos2(x, bottom)], stroke);
        if i < 9 {
            painter.text(
                egui::pos2(x + 3.0, top + 10.0),
                egui::Align2::LEFT_CENTER,
                format!("{i}×"),
                egui::FontId::monospace(9.0),
                theme::TEXT_4,
            );
        }
        i += 1;
    }
}

/// ±24 st pitch ruler in the right gutter + faint horizontal
/// gridlines at the octaves.
fn draw_pitch_ruler(painter: &egui::Painter, layout: &HeroLayout) {
    painter.line_segment(
        [
            egui::pos2(layout.plot.right() + 0.5, layout.canvas.top()),
            egui::pos2(layout.plot.right() + 0.5, layout.canvas.bottom()),
        ],
        egui::Stroke::new(1.0, theme::LINE_2),
    );
    for st in [-24i32, -12, 0, 12, 24] {
        let y = layout.y_of_st(st as f32);
        let (text_color, line_alpha) = if st == 0 {
            (theme::TEXT_3, 0.25)
        } else {
            (theme::TEXT_4, 0.10)
        };
        painter.text(
            egui::pos2(layout.plot.right() + 8.0, y),
            egui::Align2::LEFT_CENTER,
            format!("{}{st} st", if st > 0 { "+" } else { "" }),
            egui::FontId::monospace(9.0),
            text_color,
        );
        painter.line_segment(
            [
                egui::pos2(layout.plot.left(), y + 0.5),
                egui::pos2(layout.plot.right(), y + 0.5),
            ],
            egui::Stroke::new(1.0, theme::TEXT_3.gamma_multiply(line_alpha)),
        );
    }
}

/// The write head at the right plot edge: mint pulsing `WRITE` while
/// streaming; amber, pulse stopped, `HOLD` while frozen.
fn draw_write_head(painter: &egui::Painter, layout: &HeroLayout, frozen: bool, now: f64) {
    let x = layout.plot.right() - 2.0;
    let color = if frozen {
        theme::WARM.gamma_multiply(0.5)
    } else {
        let pulse = 0.55 + 0.35 * (now * 6.0).sin() as f32;
        theme::GOOD.gamma_multiply(pulse)
    };
    painter.line_segment(
        [
            egui::pos2(x, layout.plot.top() + LINE_INSET),
            egui::pos2(x, layout.plot.bottom() - LINE_INSET),
        ],
        egui::Stroke::new(2.0, color),
    );
    painter.text(
        egui::pos2(x - 6.0, layout.plot.top() + 20.0),
        egui::Align2::RIGHT_CENTER,
        if frozen { "HOLD" } else { "WRITE" },
        egui::FontId::proportional(9.0),
        if frozen { theme::WARM } else { theme::GOOD },
    );
}

/// The delay tap: accent vertical line with the ms/division flag and
/// grab dots (render only — dragging is the interaction todo).
fn draw_delay_tap(
    painter: &egui::Painter,
    layout: &HeroLayout,
    params: &GranularDelayParams,
    delay_ms: f32,
) {
    let x = layout.tap_x;
    painter.line_segment(
        [
            egui::pos2(x, layout.plot.top() + LINE_INSET),
            egui::pos2(x, layout.plot.bottom() - LINE_INSET),
        ],
        egui::Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.75)),
    );

    // Flag: division label while synced, ms otherwise.
    let label = if params.sync.value() {
        DIVISION_LABELS
            .get(params.division.value() as usize)
            .copied()
            .unwrap_or("?")
            .to_string()
    } else {
        format!("{delay_ms:.0} ms")
    };
    let font = egui::FontId::monospace(10.0);
    let galley = painter.layout_no_wrap(label, font, theme::ACCENT_SOFT);
    let w = galley.size().x + 14.0;
    let flag = egui::Rect::from_min_size(
        egui::pos2(x - w * 0.5, layout.plot.top() + 8.0),
        egui::vec2(w, 17.0),
    );
    painter.rect_filled(flag, theme::RADIUS_CHIP, theme::ACCENT.gamma_multiply(0.16));
    painter.rect_stroke(
        flag,
        theme::RADIUS_CHIP,
        egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.34)),
        egui::StrokeKind::Inside,
    );
    let text_pos = egui::pos2(flag.center().x - galley.size().x * 0.5, flag.center().y - galley.size().y * 0.5);
    painter.galley(text_pos, galley, theme::ACCENT_SOFT);

    // Grab dots at mid-height.
    let mid = layout.plot.center().y;
    for dy in [-5.0f32, 0.0, 5.0] {
        painter.rect_filled(
            egui::Rect::from_center_size(egui::pos2(x, mid + dy), egui::vec2(2.0, 2.0)),
            0.0,
            theme::ACCENT_SOFT.gamma_multiply(0.8),
        );
    }
}
