//! The plate's figure of eight (Dattorro): tank branch A as the left
//! loop, B as the right, crossing where each feeds the other. Each loop
//! is drawn as its four segments in signal order (input allpass, delay,
//! allpass, delay; the engine's `channel_energies` order, A then B), each
//! lit and thickened by its energy and labelled with its length in ms.

use std::f32::consts::{FRAC_PI_2, PI};

use plugin_gui_core::egui;

use crate::viz::FDN_CHANNELS;

use super::{display_levels, faded};
use crate::editor::theme;

/// Segment names in signal order round one branch.
const SEGMENTS: [&str; 4] = ["AP1", "D1", "AP2", "D2"];
/// Each segment's share of its loop (the delays drawn longer than the
/// allpasses, as they are).
const SPANS: [f32; 4] = [0.6, 1.4, 0.6, 1.4];
/// Points per segment polyline.
const POINTS: usize = 20;
/// Largest `|sin t cos t| / (1 + sin² t)` on the lemniscate (at
/// `sin² t = 1/3`), to normalise its height to ±1.
const LEMNISCATE_HALF_HEIGHT: f32 = 0.353_553_4;

pub(super) fn draw(
    painter: &egui::Painter,
    body: egui::Rect,
    energies: &[f32; FDN_CHANNELS],
    delays_ms: &[f32; FDN_CHANNELS],
) {
    let centre = body.center();
    let hx = body.width() * 0.5 - 14.0;
    let hy = (body.height() * 0.5 - 18.0).min(hx * 0.6).max(8.0);
    // The lemniscate of Bernoulli, scaled to ±hx by ±hy: t in
    // [π/2, 3π/2] is the left loop, [3π/2, 5π/2] the right, both
    // starting and ending at the crossing.
    let at = |t: f32| {
        let (s, c) = t.sin_cos();
        let d = 1.0 + s * s;
        centre + egui::vec2(hx * c / d, hy * s * c / d / LEMNISCATE_HALF_HEIGHT)
    };
    let levels = display_levels(energies);
    let total: f32 = SPANS.iter().sum();

    for (branch, start) in [(0usize, FRAC_PI_2), (1, FRAC_PI_2 + PI)] {
        let loop_centre = centre + egui::vec2(if branch == 0 { -0.6 } else { 0.6 } * hx, 0.0);
        // The whole loop, faint, under its lit segments.
        let outline: Vec<egui::Pos2> = (0..=4 * POINTS)
            .map(|i| at(start + PI * i as f32 / (4 * POINTS) as f32))
            .collect();
        painter.add(egui::Shape::line(
            outline,
            egui::Stroke::new(1.0, theme::BORDER),
        ));

        let mut t0 = start;
        for (k, name) in SEGMENTS.iter().enumerate() {
            let c = branch * 4 + k;
            let level = levels[c];
            let span = PI * SPANS[k] / total;
            let points: Vec<egui::Pos2> = (0..=POINTS)
                .map(|i| at(t0 + span * i as f32 / POINTS as f32))
                .collect();
            let is_delay = k % 2 == 1;
            let width = if is_delay {
                2.0 + 5.0 * level
            } else {
                1.5 + 3.0 * level
            };
            let colour = if is_delay {
                theme::ACCENT
            } else {
                theme::ACCENT_SOFT
            };
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(width, faded(colour, 0.3 + 0.7 * level)),
            ));
            // The joint where the segment starts.
            painter.circle_filled(at(t0), 2.0, theme::TEXT_DIM);

            // The label, pushed out from the loop's middle; next to the
            // crossing (both branches' first and last segments meet
            // there) it goes above or below, on its own branch's side.
            let mid = at(t0 + span * 0.5);
            let side = if branch == 0 { -1.0 } else { 1.0 };
            let (pos, align) = if (mid.x - centre.x).abs() < 0.3 * hx {
                let up = if mid.y < centre.y { -1.0 } else { 1.0 };
                let align = if branch == 0 {
                    egui::Align2::RIGHT_CENTER
                } else {
                    egui::Align2::LEFT_CENTER
                };
                (mid + egui::vec2(side * 4.0, up * 10.0), align)
            } else {
                let out = (mid - loop_centre).normalized();
                (mid + out * 12.0, egui::Align2::CENTER_CENTER)
            };
            painter.text(
                pos,
                align,
                format!("{name} {:.0}", delays_ms[c]),
                egui::FontId::monospace(8.0),
                theme::TEXT_DIM,
            );
            t0 += span;
        }
        painter.text(
            loop_centre,
            egui::Align2::CENTER_CENTER,
            if branch == 0 { "A" } else { "B" },
            egui::FontId::proportional(11.0),
            faded(
                theme::TEXT,
                0.4 + 0.6 * (levels[branch * 4..branch * 4 + 4].iter().sum::<f32>() / 4.0),
            ),
        );
    }
    // The crossing: each branch's output feeds the other's input.
    painter.circle_filled(centre, 3.5, theme::ACCENT_SOFT);
    painter.text(
        egui::pos2(body.right(), body.bottom()),
        egui::Align2::RIGHT_BOTTOM,
        "ms",
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );
}
