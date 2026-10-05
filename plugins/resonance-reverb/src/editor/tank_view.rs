//! The tank view, per algorithm (reverb-algorithms.md §4.6).
//!
//! Every engine publishes the same eight live energies and eight lengths
//! (`ReverbViz::channel_energies`, `fdn_delay_ms`); what they mean, and
//! so how they are drawn, depends on the algorithm:
//!
//! - **FDN engines** (Classic, Room, Chamber, Hall, Ambience, Nonlinear,
//!   Shimmer): an energy ring ([`ring`]), one node per line (the 16-line
//!   engines fold their lines in pairs), sized by its energy, with the
//!   feedback matrix drawn as chords and the summed energy in the centre.
//! - **Plate**: Dattorro's figure of eight ([`plate`]): two tank branches
//!   that cross-feed where the loops meet, each drawn as its four segments
//!   (allpass, delay, allpass, delay) lit by their energies.
//! - **Spring**: the echo train of both springs ([`spring`]): the onset
//!   chirp, then one echo per round trip, each chirp longer than the last,
//!   decaying at the `decay` knob's rate and lit by the live energy.

use plugin_gui_core::egui;

use crate::dsp::Algorithm;
use crate::viz::FDN_CHANNELS;

use super::theme;
use super::ReverbEditorApp;

mod plate;
mod ring;
mod spring;

/// Height of the header strip above the drawing.
const HEADER_H: f32 = 28.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, app: &ReverbEditorApp) {
    painter.rect_filled(rect, 3.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let algorithm = app.params.algorithm();
    let (title, detail) = header(algorithm);
    painter.text(
        egui::pos2(rect.left() + 10.0, rect.top() + 8.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
    painter.text(
        egui::pos2(rect.right() - 10.0, rect.top() + 8.0),
        egui::Align2::RIGHT_TOP,
        detail,
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );

    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 10.0, rect.top() + HEADER_H),
        egui::pos2(rect.right() - 10.0, rect.bottom() - 10.0),
    );
    if body.width() < 40.0 || body.height() < 40.0 {
        return;
    }
    // The viz holds whatever the last processed block's engine published:
    // after a switch the audio thread has not run yet (an inactive
    // plugin, or the next block), it is the previous algorithm's, which
    // this layout must not label as its own. Draw idle until it catches up.
    let (energies, delays_ms) = if app.viz.tank_algorithm() == Some(algorithm as i32) {
        (app.viz.read_channel_energies(), app.viz.read_fdn_delay_ms())
    } else {
        ([0.0; FDN_CHANNELS], [0.0; FDN_CHANNELS])
    };
    match algorithm {
        Algorithm::Plate => plate::draw(painter, body, &energies, &delays_ms),
        Algorithm::Spring => spring::draw(painter, body, app, &energies, &delays_ms),
        Algorithm::Classic
        | Algorithm::Room
        | Algorithm::Chamber
        | Algorithm::Hall
        | Algorithm::Ambience
        | Algorithm::Nonlinear
        | Algorithm::Shimmer => ring::draw(painter, body, &energies, &delays_ms),
    }
}

/// The panel's title and the detail in its top-right corner.
fn header(algorithm: Algorithm) -> (&'static str, &'static str) {
    match algorithm {
        Algorithm::Classic => ("FDN TANK", "8 lines · Householder"),
        Algorithm::Ambience => ("FDN TANK", "8 lines"),
        Algorithm::Room
        | Algorithm::Chamber
        | Algorithm::Hall
        | Algorithm::Nonlinear
        | Algorithm::Shimmer => ("FDN TANK", "16 lines · in pairs"),
        Algorithm::Plate => ("PLATE TANK", "figure of eight"),
        Algorithm::Spring => ("SPRINGS", "echo train · A / B"),
    }
}

/// The energies for display, `0..=1`: normalised against the loudest
/// channel so the view fills its range at any input level (with a floor,
/// so near-silence stays small), with a mild curve so a channel at 20 %
/// of the peak is still visible.
fn display_levels(energies: &[f32; FDN_CHANNELS]) -> [f32; FDN_CHANNELS] {
    let peak = energies.iter().copied().fold(0.0f32, f32::max).max(0.05);
    std::array::from_fn(|c| (energies[c] / peak).clamp(0.0, 1.0).powf(0.55))
}

/// The summed energy as one `0..=1` level (the old "mix bus" bar).
fn overall_level(energies: &[f32; FDN_CHANNELS]) -> f32 {
    let peak = energies.iter().copied().fold(0.0f32, f32::max).max(0.05);
    let sum: f32 = energies.iter().sum();
    (sum / (peak * FDN_CHANNELS as f32 * 0.5))
        .clamp(0.0, 1.0)
        .powf(0.6)
}

/// `color` at `alpha` (`0..=1`) of its own opacity.
fn faded(color: egui::Color32, alpha: f32) -> egui::Color32 {
    color.gamma_multiply(alpha.clamp(0.0, 1.0))
}
