//! The granular editor fits its own window (code review PUX-08).
//!
//! The control strip is six fixed-width groups in one row, 1300-odd px
//! of it, while the window allowed itself down to 1000 px: between 1000
//! and ~1303 px OUTPUT (Mix, Quality, Freeze) and part of SPACE were
//! cut off. The minimum width is now derived from the strip itself.
//! These are headless frames of the real editor app, checked control by
//! control at the minimum, default and tiled sizes.
#![cfg(feature = "editor")]

use resonance_granular_delay::editor::controls::{control_kind, ControlKind, GROUPS};
use resonance_granular_delay::editor::{headless_editor, MIN_H, MIN_W, WINDOW_H, WINDOW_W};
use resonance_granular_delay::params::GranularDelayParams;
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{Param, ResonancePlugin};

fn sizes() -> [(f32, f32); 3] {
    [
        (MIN_W as f32, MIN_H as f32),
        (WINDOW_W as f32, WINDOW_H as f32),
        (1571.0, 856.0),
    ]
}

/// The params drawn in the default state (SYNC on swaps Time for the
/// division stepper; Quantize ≠ Scale hides Root/Scale; PER-BEAT off
/// hides the density division) — each must be on screen.
fn drawn_by_default(params: &GranularDelayParams) -> Vec<String> {
    let mut ids = Vec::new();
    for group in GROUPS {
        for &i in group.params {
            let id = params.param_at(i).id().to_string();
            let hidden = match id.as_str() {
                "time_ms" => params.sync.value(),
                "division" => !params.sync.value(),
                "root" | "scale" => params.pitch_quantize.value() != 2,
                "density_division" => !params.density_sync.value(),
                "density_hz" => params.density_sync.value(),
                _ => false,
            };
            if !hidden {
                ids.push(id);
            }
        }
    }
    ids
}

#[test]
fn every_control_is_inside_the_window_at_every_size() {
    let plugin = ResonanceGranularDelay::new();
    let ids = drawn_by_default(&plugin.params);
    assert!(ids.len() > 20, "the walk found only {} controls", ids.len());
    for size in sizes() {
        let mut editor = headless_editor(&plugin, size);
        let frame = editor.settled();
        for id in &ids {
            assert!(frame.widget(id).is_some(), "`{id}` is not drawn at {size:?}");
        }
        let hidden = frame.hidden_widgets(1.0);
        assert!(hidden.is_empty(), "controls not fully visible at {size:?}: {hidden:#?}");
    }
}

#[test]
fn the_old_minimum_width_really_clipped() {
    // Guard the guard: at the old 1000 px minimum the strip's tail is
    // off the window, so the check above would have caught PUX-08.
    let plugin = ResonanceGranularDelay::new();
    let mut editor = headless_editor(&plugin, (1000.0, MIN_H as f32));
    let frame = editor.settled();
    let hidden = frame.hidden_widgets(1.0);
    assert!(
        hidden.iter().any(|(id, _)| id == "mix" || id == "quality"),
        "expected OUTPUT to be clipped at 1000 px: {hidden:#?}"
    );
    assert!(MIN_W > 1000);
}

#[test]
fn every_knob_kind_has_a_typed_float_param() {
    // The knobs bind through the param's own skew, which needs the
    // typed `FloatParam` behind each knob index.
    let params = GranularDelayParams::default();
    for group in GROUPS {
        for &i in group.params {
            if let ControlKind::Knob = control_kind(i) {
                let p = resonance_granular_delay::editor::widgets::float_at(&params, i)
                    .unwrap_or_else(|| panic!("knob {i} has no FloatParam"));
                assert_eq!(p.id(), params.param_at(i).id());
            }
        }
    }
}
