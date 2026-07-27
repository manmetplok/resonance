//! Editor control-surface data tests (ba todo #1079): the §9 group
//! table covers the full declared parameter surface exactly once, the
//! widget mapping is consistent with each parameter's declared range,
//! and every cached static option list matches its parameter and its
//! source of truth.

#![cfg(feature = "editor")]

use resonance_granular_delay::editor::controls::{
    control_kind, ControlKind, GROUPS, GROUP_ROWS, ROOT_LABELS, SCALE_LABELS,
};
use resonance_granular_delay::params::{GranularDelayParams, PARAM_COUNT};
use resonance_music_theory::Mode;
use resonance_plugin::Param;

#[test]
fn groups_cover_every_param_exactly_once() {
    let mut seen = vec![0usize; PARAM_COUNT];
    for group in GROUPS {
        for &index in group.params {
            assert!(index < PARAM_COUNT, "group {} has out-of-range index {index}", group.name);
            seen[index] += 1;
        }
    }
    for (index, &count) in seen.iter().enumerate() {
        assert_eq!(
            count, 1,
            "param index {index} appears {count} times across the groups (expected exactly 1)"
        );
    }
}

#[test]
fn group_rows_cover_every_group_exactly_once() {
    let mut seen = vec![0usize; GROUPS.len()];
    for row in GROUP_ROWS {
        for &g in row.iter() {
            assert!(g < GROUPS.len(), "row references missing group {g}");
            seen[g] += 1;
        }
    }
    assert!(seen.iter().all(|&c| c == 1), "group row layout skips or repeats a group: {seen:?}");
}

#[test]
fn widget_kinds_match_declared_param_ranges() {
    let params = GranularDelayParams::default();
    for index in 0..PARAM_COUNT {
        let p = params.param_at(index);
        match control_kind(index) {
            ControlKind::Choice(labels) => {
                assert_eq!(p.min_plain(), 0.0, "choice param {} must start at 0", p.id());
                assert_eq!(
                    labels.len() as f64,
                    p.max_plain() + 1.0,
                    "label list for {} has {} entries but the param range is 0..={}",
                    p.id(),
                    labels.len(),
                    p.max_plain()
                );
            }
            ControlKind::Toggle => {
                assert_eq!(
                    (p.min_plain(), p.max_plain()),
                    (0.0, 1.0),
                    "toggle param {} is not boolean-ranged",
                    p.id()
                );
            }
            ControlKind::Knob => {
                assert!(
                    p.max_plain() > p.min_plain(),
                    "knob param {} has an empty range",
                    p.id()
                );
            }
        }
    }
}

#[test]
fn toggles_are_exactly_the_boolean_params() {
    let expected = [0usize, 6, 9, 19]; // sync, fb_pitch, density_sync, freeze
    for index in 0..PARAM_COUNT {
        let is_toggle = matches!(control_kind(index), ControlKind::Toggle);
        assert_eq!(
            is_toggle,
            expected.contains(&index),
            "widget kind mismatch at param index {index}"
        );
    }
}

#[test]
fn scale_labels_match_music_theory_modes() {
    assert_eq!(SCALE_LABELS.len(), Mode::ALL.len());
    for (label, mode) in SCALE_LABELS.iter().zip(Mode::ALL.iter()) {
        assert_eq!(
            label.to_lowercase(),
            mode.as_str(),
            "scale label {label} out of sync with Mode::ALL"
        );
    }
}

#[test]
fn root_labels_cover_the_twelve_pitch_classes() {
    assert_eq!(ROOT_LABELS.len(), 12);
    // Chromatic ascent: every label unique.
    let mut set = std::collections::HashSet::new();
    for label in ROOT_LABELS {
        assert!(set.insert(*label), "duplicate root label {label}");
    }
}
