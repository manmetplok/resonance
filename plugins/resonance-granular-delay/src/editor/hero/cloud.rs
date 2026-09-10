//! The live pitch layer of the hero band (ba todo #1143, design doc
//! #264 req-1): the scale lanes the quantizer snaps to and the grain
//! cloud itself, painted into the seam the frame leaves between the
//! backdrop and the heads.

use plugin_gui_core::egui;

use crate::params::GranularDelayParams;
use crate::quantize::{mode_from_index, quantize_transpose, root_from_index, PitchQuantize};
use crate::viz::GrainSnapshot;

use super::super::controls::ROOT_LABELS;
use super::super::theme;
use super::layout::{HeroLayout, PITCH_RANGE_ST};

/// Scale lanes (ba todo #1143, req-1; prototype 'scale' state): with
/// Quantize == SCALE, horizontal accent lanes at every allowed scale
/// degree within the ±24 st ruler, computed through the exact
/// quantizer the DSP snaps grains with (`crate::quantize`) — capsules
/// land on the lanes because both sides share the lattice. Small
/// 'D MINOR LANES' caption bottom-left.
pub(super) fn draw_scale_lanes(
    painter: &egui::Painter,
    layout: &HeroLayout,
    params: &GranularDelayParams,
) {
    if params.pitch_quantize.value() != 2 {
        return;
    }
    let root = params.root.value();
    let scale = resonance_music_theory::Scale::new(
        root_from_index(root),
        mode_from_index(params.scale.value()),
    );
    let stroke = egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.13));
    let range = PITCH_RANGE_ST as i32;
    for st in -range..=range {
        // A lane exists where the quantizer is a fixed point.
        let snapped = quantize_transpose(st as f32, PitchQuantize::Scale, scale);
        if (snapped - st as f32).abs() < 0.01 {
            let y = layout.y_of_st(st as f32);
            painter.line_segment(
                [
                    egui::pos2(layout.plot.left(), y + 0.5),
                    egui::pos2(layout.plot.right(), y + 0.5),
                ],
                stroke,
            );
        }
    }
    let root_label = ROOT_LABELS
        .get(root.max(0) as usize % 12)
        .copied()
        .unwrap_or("?");
    painter.text(
        egui::pos2(layout.plot.left() + 12.0, layout.plot.bottom() - 34.0),
        egui::Align2::LEFT_CENTER,
        format!(
            "{root_label} {} LANES",
            scale.mode.as_str().to_uppercase()
        ),
        egui::FontId::proportional(9.0),
        theme::ACCENT_SOFT.gamma_multiply(0.55),
    );
}

/// The live grain cloud (ba todo #1143, req-1): soft capsules at
/// x = buffer position / y = pitch, width = grain size, alpha = the
/// snapshot level (the DSP's Texture-shaped envelope value at this
/// instant). Reversed grains carry a warm left-edge taper; feedback
/// ghosts (generation ≥ 1) draw in the plain accent token, further
/// dimmed per generation on top of the loop-gain scaling the
/// publisher applied (and already positioned one delay further back
/// per generation, pitch including recirculation transposition);
/// PSOLA voices draw as tight mint period-slivers. Pure iteration
/// over the caller's stack buffer — no heap allocation.
pub(super) fn draw_grain_cloud(
    painter: &egui::Painter,
    layout: &HeroLayout,
    grains: &[GrainSnapshot],
    texture: f32,
) {
    let plot = layout.plot;
    for g in grains {
        // Ghosts: flat extra dim on top of the publisher's fb^gen level
        // scaling, deepening slightly per generation.
        let ghost_dim = match g.generation {
            0 => 0.95,
            1 => 0.55,
            2 => 0.45,
            _ => 0.35,
        };
        let alpha = (g.level * ghost_dim).clamp(0.0, 1.0);
        if alpha < 0.01 {
            continue;
        }
        let x = layout.x_of_ms(g.position_ms);
        let w = ((g.size_ms * 0.001 / layout.window_seconds) * plot.width()).max(10.0);
        if x + w * 0.5 < plot.left() || x - w * 0.5 > plot.right() {
            continue; // fully outside the visible window
        }
        let y = layout.y_of_st(g.pitch_semitones.clamp(-26.0, 26.0));
        let color = if g.voiced {
            theme::GOOD
        } else if g.generation > 0 {
            theme::ACCENT
        } else {
            theme::ACCENT_SOFT
        };
        // Voiced period-slivers are tight; cloud capsules thicken with
        // the Texture window shape.
        let h = if g.voiced { 5.0 } else { 8.0 + texture * 5.0 };
        let rect = egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(w, h))
            .intersect(plot);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            continue;
        }
        painter.rect_filled(rect, h * 0.5, color.gamma_multiply(alpha));

        // Reversed grains: warm marker on the leading (left) edge —
        // the grain plays toward older material. Drawn as two stacked
        // strips (wide + narrow) approximating the prototype's taper
        // without building a per-grain polygon (no heap allocation).
        if g.reversed {
            let lx = x - w * 0.5;
            if lx >= plot.left() - 1.0 && lx <= plot.right() {
                let warm = theme::WARM.gamma_multiply(alpha);
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(lx - 1.0, y - h * 0.5),
                        egui::vec2(3.0, h),
                    )
                    .intersect(plot),
                    1.0,
                    warm,
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(lx + 2.0, y - h * 0.25),
                        egui::vec2(2.5, h * 0.5),
                    )
                    .intersect(plot),
                    1.0,
                    warm,
                );
            }
        }
    }
}
