//! Editor layout-table tests (ba todo #1275).
//!
//! The gate's editor is data-driven from `editor::GROUPS`: a knob that
//! is not listed there is simply never drawn, which is exactly the kind
//! of silent gap the plugin audit found elsewhere. These tests pin the
//! table to the declared parameter surface so adding a parameter
//! without giving it a home fails here rather than in a user's window.

#![cfg(feature = "editor")]

use resonance_gate::editor::GROUPS;
use resonance_gate::params::{GateParams, PARAM_COUNT};

#[test]
fn groups_cover_every_param_exactly_once() {
    let mut seen = [0usize; PARAM_COUNT];
    for group in GROUPS {
        for &index in group.params {
            assert!(
                index < PARAM_COUNT,
                "group {} has out-of-range index {index}",
                group.caption
            );
            seen[index] += 1;
        }
    }
    let params = GateParams::default();
    for (index, &count) in seen.iter().enumerate() {
        assert_eq!(
            count,
            1,
            "param '{}' (index {index}) appears {count} times across the editor groups \
             (expected exactly 1)",
            params.param_at(index).id()
        );
    }
}

#[test]
fn every_group_is_captioned_and_populated() {
    assert!(!GROUPS.is_empty(), "the editor draws no knob groups at all");
    for group in GROUPS {
        assert!(!group.caption.is_empty(), "a knob group has no caption");
        assert!(
            !group.params.is_empty(),
            "group '{}' draws no knobs",
            group.caption
        );
    }
}
