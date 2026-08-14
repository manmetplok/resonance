//! Hero canvas gestures (ba todo #1144, design doc #264 req-2).
//!
//! Input only: it reads the pointer against a [`HeroLayout`] and writes
//! parameters through `Param::set_plain` — the same GUI→host path as
//! the strip knobs, so host automation records the gestures. The tempo
//! grid it snaps to comes from [`crate::sync`]; nothing musical is
//! derived here (ba todo #1265).

use egui::Ui;
use resonance_plugin::Param;
use wayland_plugin_gui::egui;

use crate::params::GranularDelayParams;
use crate::sync::nearest_division;
use crate::viz::GranularViz;

use super::layout::HeroLayout;

/// Which hero gesture is in flight (ba todo #1144). Latched at drag
/// start so the tap grab wins over the cloud drag for the whole
/// gesture even when the pointer leaves the hit zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeroDrag {
    /// Horizontal delay-tap drag (time / division).
    Tap,
    /// Vertical cloud drag (pitch).
    Cloud,
}

/// Canvas interactions (ba todo #1144, design doc #264 req-2): tap
/// drag ⇄ delay time (snapping to the division grid while synced),
/// cloud drag ⇅ Pitch, scroll = Density, double-click on the tap
/// resets the active time param to its default. All writes go through
/// `Param::set_plain` — the same GUI→host path as the strip knobs —
/// so host automation records the gestures. Interaction is scoped to
/// the plot rect, so the header/strip widgets are untouched.
pub fn interact(
    ui: &mut Ui,
    params: &GranularDelayParams,
    viz: &GranularViz,
    layout: &HeroLayout,
    drag: &mut Option<HeroDrag>,
) {
    let response = ui.interact(
        layout.plot,
        ui.id().with("hero_canvas"),
        egui::Sense::click_and_drag(),
    );
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let over_tap = pointer.is_some_and(|p| layout.tap_hit.contains(p));

    // Latch the gesture at drag start: the tap hit-zone wins over the
    // cloud drag when they overlap.
    if response.drag_started() {
        *drag = Some(if over_tap { HeroDrag::Tap } else { HeroDrag::Cloud });
    }
    if response.drag_stopped() {
        *drag = None;
    }

    // Cursor affordances: horizontal-resize near/while dragging the
    // tap, grab over the cloud body, grabbing while dragging it.
    let cursor = match (*drag, over_tap, response.hovered()) {
        (Some(HeroDrag::Tap), _, _) => Some(egui::CursorIcon::ResizeHorizontal),
        (Some(HeroDrag::Cloud), _, _) => Some(egui::CursorIcon::Grabbing),
        (None, true, _) => Some(egui::CursorIcon::ResizeHorizontal),
        (None, false, true) => Some(egui::CursorIcon::Grab),
        _ => None,
    };
    if let Some(cursor) = cursor {
        ui.ctx().set_cursor_icon(cursor);
    }

    // Double-click on the tap: reset the active time param (free-run
    // time in ms, or the division while synced) to its default.
    if response.double_clicked() && over_tap {
        let p: &dyn resonance_plugin::Param = if params.sync.value() {
            &params.division
        } else {
            &params.time_ms
        };
        p.set_plain(p.default_plain());
        return;
    }

    match (*drag, pointer) {
        (Some(HeroDrag::Tap), Some(pos)) => {
            let target_ms = layout.ms_of_x(pos.x);
            let bpm = viz.read_bpm();
            if params.sync.value() && bpm > 0.0 {
                // Snap to the division grid: step the division param
                // to the division nearest the pointer time (the same
                // grid the tick marks draw), and let the flag readout
                // follow live.
                let best = nearest_division(bpm, target_ms);
                if best as i32 != params.division.value() {
                    params.division.set_plain(best as f64);
                }
            } else {
                // Free-run: x maps to delay ms within the param range.
                let p = &params.time_ms;
                let ms = f64::from(target_ms).clamp(p.min_plain(), p.max_plain());
                p.set_plain(ms);
            }
        }
        (Some(HeroDrag::Cloud), Some(pos)) => {
            // Vertical drag maps the pointer's ruler position to the
            // bipolar Pitch param (±24 st); with Quantize == SCALE the
            // DSP's spawn quantization makes grains land on the lanes.
            let p = &params.pitch;
            let st = f64::from(layout.st_of_y(pos.y)).clamp(p.min_plain(), p.max_plain());
            p.set_plain(st);
        }
        _ => {}
    }

    // Scroll over the canvas: fine multiplicative Density steps.
    // `density_sync` (PER-BEAT) has no separate synced-density param
    // in params.rs (its DSP is a declared TODO), so the free-run
    // density param is the single scroll target in both modes.
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let p = &params.density_hz;
            let factor = f64::from((-scroll * 0.0015).exp());
            let next = (p.get_plain() * factor).clamp(p.min_plain(), p.max_plain());
            p.set_plain(next);
        }
    }
}
