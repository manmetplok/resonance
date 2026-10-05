//! The FDN energy ring: one node per (folded) line round a circle, each
//! sized and lit by its energy and labelled with its length in ms. Every
//! line feeds every other through the feedback matrix, drawn as faint
//! chords that brighten with the tank's total energy, which also fills
//! the centre.

use std::f32::consts::{FRAC_PI_2, TAU};

use plugin_gui_core::egui;

use crate::viz::FDN_CHANNELS;

use super::{display_levels, faded, overall_level};
use crate::editor::theme;

/// Room left round the ring for the length labels.
const LABEL_MARGIN: f32 = 22.0;

pub(super) fn draw(
    painter: &egui::Painter,
    body: egui::Rect,
    energies: &[f32; FDN_CHANNELS],
    delays_ms: &[f32; FDN_CHANNELS],
) {
    let centre = body.center();
    let radius = (body.width().min(body.height()) * 0.5 - LABEL_MARGIN).max(8.0);
    let levels = display_levels(energies);
    let overall = overall_level(energies);
    let node = |c: usize| {
        let angle = -FRAC_PI_2 + c as f32 * TAU / FDN_CHANNELS as f32;
        (
            angle,
            centre + radius * egui::vec2(angle.cos(), angle.sin()),
        )
    };

    // The feedback matrix: every line into every other.
    let chord = egui::Stroke::new(0.5, faded(theme::ACCENT_SOFT, 0.08 + 0.3 * overall));
    for i in 0..FDN_CHANNELS {
        for j in i + 1..FDN_CHANNELS {
            painter.line_segment([node(i).1, node(j).1], chord);
        }
    }
    painter.circle_stroke(centre, radius, egui::Stroke::new(1.0, theme::BORDER));

    // The summed energy in the centre.
    painter.circle_filled(centre, radius * 0.28 * overall, theme::ACCENT_DIM);
    painter.circle_stroke(
        centre,
        radius * 0.28 * overall.max(0.05),
        egui::Stroke::new(1.0, faded(theme::ACCENT, 0.4 + 0.6 * overall)),
    );

    let node_r = (radius * 0.16).clamp(4.0, 14.0);
    for (c, &level) in levels.iter().enumerate() {
        let (angle, p) = node(c);
        // A spoke from the centre, as long as the line is loud.
        painter.line_segment(
            [centre, centre + (p - centre) * level],
            egui::Stroke::new(2.0, faded(theme::ACCENT, 0.25 + 0.5 * level)),
        );
        painter.circle_filled(p, node_r, theme::BG);
        painter.circle_filled(
            p,
            node_r * (0.3 + 0.7 * level),
            faded(theme::ACCENT, 0.3 + 0.7 * level),
        );
        painter.circle_stroke(p, node_r, egui::Stroke::new(1.0, theme::ACCENT_SOFT));
        let label = centre + (radius + LABEL_MARGIN * 0.55) * egui::vec2(angle.cos(), angle.sin());
        painter.text(
            label,
            egui::Align2::CENTER_CENTER,
            format!("{:.0}", delays_ms[c]),
            egui::FontId::monospace(9.0),
            theme::TEXT_DIM,
        );
    }
    painter.text(
        egui::pos2(body.right(), body.bottom()),
        egui::Align2::RIGHT_BOTTOM,
        "ms",
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );
}
