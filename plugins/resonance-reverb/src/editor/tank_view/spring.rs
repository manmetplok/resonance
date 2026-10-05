//! The springs' echo trains: spring A in the upper lane, B in the lower.
//! Each lane draws the onset (the input's first pass, which the ER/tail
//! balance treats as the early part) and then one echo per round trip,
//! each a falling chirp (treble first, bass trailing) that lengthens with
//! every pass as the dispersion piles up, and quieter by the `decay`
//! knob's T60. `spring_tension` sets how long the chirps are,
//! `spring_drip` stretches the onset's; the live energy of each spring
//! lights its lane.

use plugin_gui_core::egui;

use crate::editor::ReverbEditorApp;
use crate::viz::FDN_CHANNELS;

use super::{display_levels, faded};
use crate::editor::theme;

/// Round trips shown after the onset.
const ECHOES: usize = 5;
/// Polyline points per chirp.
const POINTS: usize = 48;
/// The round trip at `size` 0 and 1, ms (the engine's map), for before
/// the engine has published its own.
const ROUND_TRIP_MS: (f32, f32) = (30.0, 90.0);
/// Spring B's length against A's (the engine's).
const B_LENGTH: f32 = 1.13;

pub(super) fn draw(
    painter: &egui::Painter,
    body: egui::Rect,
    app: &ReverbEditorApp,
    energies: &[f32; FDN_CHANNELS],
    delays_ms: &[f32; FDN_CHANNELS],
) {
    let params = &app.params;
    let decay_s = app
        .viz
        .synced_decay_s()
        .unwrap_or_else(|| params.decay.value())
        .max(0.05);
    let tension = params.spring_tension.value().clamp(0.0, 1.0);
    let drip = params.spring_drip.value().clamp(0.0, 1.0);
    // The round trips the engine publishes (A on the even slots, B on the
    // odd), or the size knob's before it has run.
    let fallback = {
        let (lo, hi) = ROUND_TRIP_MS;
        lo * (hi / lo).powf(params.size.value().clamp(0.0, 1.0))
    };
    let trip = |k: usize, scale: f32| {
        let ms = delays_ms[k];
        if ms > 1.0 {
            ms
        } else {
            fallback * scale
        }
    };
    let trips = [trip(0, 1.0), trip(1, B_LENGTH)];
    let levels = display_levels(energies);
    // The view spans the onset and ECHOES round trips of the longer spring.
    let window_ms = trips[1].max(trips[0]) * (ECHOES as f32 + 0.6);

    let lane_h = body.height() * 0.5;
    for (s, (&round_trip, name)) in trips.iter().zip(["A", "B"]).enumerate() {
        let lane = egui::Rect::from_min_size(
            egui::pos2(body.left(), body.top() + s as f32 * lane_h),
            egui::vec2(body.width(), lane_h),
        );
        let live = levels[s];
        draw_lane(
            painter,
            lane.shrink2(egui::vec2(0.0, 4.0)),
            Lane {
                round_trip,
                window_ms,
                decay_s,
                tension,
                drip,
                live,
            },
        );
        painter.text(
            lane.left_top() + egui::vec2(2.0, 2.0),
            egui::Align2::LEFT_TOP,
            format!("{name} {round_trip:.0} ms"),
            egui::FontId::monospace(9.0),
            faded(theme::TEXT, 0.45 + 0.55 * live),
        );
    }
}

struct Lane {
    round_trip: f32,
    window_ms: f32,
    decay_s: f32,
    tension: f32,
    drip: f32,
    live: f32,
}

fn draw_lane(painter: &egui::Painter, lane: egui::Rect, l: Lane) {
    let axis_y = lane.center().y;
    let half = lane.height() * 0.42;
    let x_of = |ms: f32| lane.left() + lane.width() * (ms / l.window_ms).clamp(0.0, 1.0);
    painter.line_segment(
        [
            egui::pos2(lane.left(), axis_y),
            egui::pos2(lane.right(), axis_y),
        ],
        egui::Stroke::new(0.5, theme::BORDER),
    );

    // The onset's chirp is the shortest; each pass adds the cascade's
    // dispersion again. Tension stretches them all.
    let base_ms = (2.0 + 8.0 * l.tension) * (1.0 + 0.6 * l.drip);
    let per_pass_ms = 1.5 + 9.0 * l.tension;
    for k in 0..=ECHOES {
        let start = k as f32 * l.round_trip;
        if start >= l.window_ms {
            break;
        }
        let length = (base_ms + per_pass_ms * k as f32).min(0.9 * l.round_trip);
        // 10^(−3 t / T60) at this echo.
        let gain = 10f32.powf(-3.0 * start * 0.001 / l.decay_s);
        if gain < 1e-3 {
            break;
        }
        let colour = if k == 0 {
            theme::WARM
        } else {
            theme::ACCENT_SOFT
        };
        let points: Vec<egui::Pos2> = (0..=POINTS)
            .map(|i| {
                let u = i as f32 / POINTS as f32;
                // A falling chirp: the phase advances fast first, slowly
                // at the end (the bass trailing).
                let phase = std::f32::consts::TAU * (5.0 + 3.0 * l.tension) * (2.0 * u - u * u);
                let envelope = (u * 12.0).min(1.0) * (1.0 - u).powf(1.5);
                let y = axis_y - half * gain * envelope * phase.sin();
                egui::pos2(x_of(start + length * u), y)
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.2, faded(colour, (0.35 + 0.65 * l.live) * gain.sqrt())),
        ));
        // A tick where each echo leaves.
        let x = x_of(start);
        painter.line_segment(
            [
                egui::pos2(x, axis_y + half * 0.85),
                egui::pos2(x, axis_y + half),
            ],
            egui::Stroke::new(1.0, theme::TEXT_DIM),
        );
    }
}
