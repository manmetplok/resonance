//! Editor layout table and widget binding (the dual-surface rule: every
//! CLAP parameter is also in the GUI).
//!
//! The editor draws exactly what `editor::GROUPS` lists, through the one
//! call site in `editor/widgets.rs`, choosing the widget by the
//! parameter's declared type. These tests pin the table to the declared
//! surface and the binding to the parameters, so adding a parameter
//! without giving it a control fails here. (No plugin editor in the
//! fleet has pixel snapshots; the layout table is what is pinned.)

#![cfg(feature = "editor")]

use resonance_stereo::editor::scope::{gonio_xy, strip_fraction};
use resonance_stereo::editor::{control_kind, ControlKind, GROUPS};
use resonance_stereo::params::{StereoParams, PARAM_COUNT};

const WIDGET_SRC: &str = include_str!("../src/editor/widgets.rs");

#[test]
fn groups_cover_every_param_exactly_once() {
    let params = StereoParams::default();
    let mut seen = [0usize; PARAM_COUNT];
    for group in GROUPS {
        assert!(!group.caption.is_empty() && !group.params.is_empty());
        for &i in group.params {
            assert!(i < PARAM_COUNT, "group {} has out-of-range index {i}", group.caption);
            seen[i] += 1;
        }
    }
    for (i, &count) in seen.iter().enumerate() {
        assert_eq!(
            count,
            1,
            "param '{}' appears {count} times across the editor groups (expected 1)",
            params.param_at(i).id()
        );
    }
}

#[test]
fn widget_kinds_match_the_declared_params() {
    let params = StereoParams::default();
    for i in 0..PARAM_COUNT {
        let p = params.param_at(i);
        match control_kind(&params, i) {
            ControlKind::Choice(labels) => {
                assert!(p.is_stepped(), "{} is not stepped", p.id());
                assert_eq!(p.min_plain(), 0.0);
                assert_eq!(
                    labels.len() as f64,
                    p.max_plain() + 1.0,
                    "labels for {} do not cover its range",
                    p.id()
                );
                for (k, label) in labels.iter().enumerate() {
                    assert_eq!(&p.display(k as f64), label);
                }
            }
            ControlKind::Toggle => {
                assert_eq!((p.min_plain(), p.max_plain()), (0.0, 1.0), "{}", p.id());
            }
            ControlKind::Knob => {
                assert!(!p.is_stepped() && p.max_plain() > p.min_plain(), "{}", p.id());
            }
        }
    }
}

#[test]
fn the_editor_binds_controls_only_through_the_param_helpers() {
    assert_eq!(WIDGET_SRC.matches("editor_widgets::float_knob(").count(), 1);
    assert_eq!(WIDGET_SRC.matches("editor_widgets::bool_checkbox(").count(), 1);
    assert_eq!(WIDGET_SRC.matches("widgets::segmented(").count(), 1);
    assert!(
        !WIDGET_SRC.contains("widgets::knob("),
        "the raw knob takes its range as arguments — restating the param"
    );
}

#[test]
fn every_value_the_host_displays_parses_back() {
    // Typed entry and host text-to-value go through the same parse.
    let params = StereoParams::default();
    for i in 0..PARAM_COUNT {
        let p = params.param_at(i);
        for frac in [0.0, 0.25, 0.5, 0.9, 1.0] {
            let v = p.min_plain() + (p.max_plain() - p.min_plain()) * frac;
            p.set_plain(v);
            let v = p.get_plain();
            let text = p.display(v);
            let back = p
                .parse(&text)
                .unwrap_or_else(|| panic!("'{}' cannot parse its own display {text:?}", p.id()));
            p.set_plain(back);
            assert_eq!(p.display(p.get_plain()), text, "'{}' round trip of {text:?}", p.id());
        }
    }
}

#[test]
fn the_goniometer_maps_mono_up_and_hard_pans_to_the_diagonals() {
    let (x, y) = gonio_xy(0.5, 0.5);
    assert_eq!(x, 0.0);
    assert!(y > 0.0, "mono is a vertical line");
    let (x, y) = gonio_xy(0.5, -0.5);
    assert!(x < 0.0 && y == 0.0, "antiphase is horizontal");
    let (xl, yl) = gonio_xy(1.0, 0.0);
    let (xr, yr) = gonio_xy(0.0, 1.0);
    assert!(xl < 0.0 && xr > 0.0 && (yl - yr).abs() < 1e-6, "L left, R right");
    assert!(((xl * xl + yl * yl).sqrt() - 1.0).abs() < 1e-6, "full-scale pan on the unit circle");
    assert_eq!(strip_fraction(-1.0), 0.0);
    assert_eq!(strip_fraction(1.0), 1.0);
    assert_eq!(strip_fraction(f32::NAN), 0.5);
}
