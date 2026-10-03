//! The delay editor fits its own window (code review PUX-03).
//!
//! The strip used to be one unwrapped row of 22 knobs, ~1710 px wide:
//! Gate and Duck were cut off at the default 1200 px, Duck Threshold and
//! Release even at the 1571 px a tiling compositor gives every editor on
//! this machine, and half the strip at the 900 px minimum. These are
//! headless CPU-only frames of the real editor app, checked control by
//! control: every parameter's control must be drawn, inside the clip it
//! was painted under and inside the window.
#![cfg(feature = "editor")]

use resonance_delay::editor::controls::{Item, Switch, GROUPS};
use resonance_delay::editor::{headless_editor, MIN_H, MIN_W, WINDOW_H, WINDOW_W};
use resonance_delay::params::{DelayParams, PARAM_COUNT};
use resonance_delay::ResonanceDelay;
use resonance_plugin::{Param, ResonancePlugin};

/// Minimum, default, and the tiled size (CLAUDE.md).
fn sizes() -> [(f32, f32); 3] {
    [
        (MIN_W as f32, MIN_H as f32),
        (WINDOW_W as f32, WINDOW_H as f32),
        (1571.0, 856.0),
    ]
}

/// Every param id the strip's table draws a control for, in order.
fn table_ids(params: &DelayParams) -> Vec<String> {
    let mut ids = Vec::new();
    for group in GROUPS {
        for item in group.items {
            match item {
                Item::Knob(at, _) => ids.push(at(params).id().to_string()),
                Item::Switches(switches) => {
                    for s in *switches {
                        ids.push(match s {
                            Switch::Toggle(at, _) => at(params).id().to_string(),
                            Switch::Segments(at, _) | Switch::Division(at) => {
                                at(params).id().to_string()
                            }
                        });
                    }
                }
            }
        }
    }
    ids
}

#[test]
fn the_table_covers_every_param_exactly_once() {
    let params = DelayParams::default();
    let ids = table_ids(&params);
    for i in 0..PARAM_COUNT {
        let id = params.param_at(i).id();
        let n = ids.iter().filter(|x| *x == id).count();
        assert_eq!(n, 1, "param `{id}` has {n} controls in the strip (want exactly 1)");
    }
    assert_eq!(ids.len(), PARAM_COUNT, "the strip draws a control for an unknown param");
}

#[test]
fn every_control_is_inside_the_window_at_every_size() {
    let plugin = ResonanceDelay::new();
    let params = DelayParams::default();
    let ids = table_ids(&params);
    for size in sizes() {
        let mut editor = headless_editor(&plugin, size);
        let frame = editor.settled();
        for id in &ids {
            assert!(
                frame.widget(id).is_some(),
                "`{id}` is not drawn at {size:?} (drawn: {:?})",
                frame.widgets.iter().map(|w| &w.name).collect::<Vec<_>>()
            );
        }
        let hidden = frame.hidden_widgets(1.0);
        assert!(hidden.is_empty(), "controls not fully visible at {size:?}: {hidden:#?}");
    }
}

#[test]
fn the_echo_view_keeps_room_at_the_minimum_size() {
    // The strip wraps rather than eating the window: at the minimum size
    // the echo view still gets a usable band above it.
    let strip = resonance_delay::editor::controls::strip_height(MIN_W as f32);
    let header = 42.0;
    assert!(
        MIN_H as f32 - header - strip >= 60.0,
        "the strip ({strip} px) leaves the echo view under 60 px at {MIN_W}x{MIN_H}"
    );
}
