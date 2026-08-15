//! Division labelling (ba todo #1273): `division` and `gate_rate` are
//! indices into one label table, and that table must be what every
//! consumer of `Param::display` sees — the editor knob, the host's
//! automation lane, and any future parameter-text surface. A raw "7" is
//! the bug this pins shut.

use resonance_delay::params::DelayParams;
use resonance_delay::sync::{division_from_label, division_label, DIVISION_LABELS};
use resonance_delay::ResonanceDelay;
use resonance_plugin::ResonancePlugin;

/// The two parameters that carry a division, by param index.
const DIVISION_PARAMS: &[(usize, &str)] = &[(1, "division"), (15, "gate_rate")];

#[test]
fn division_params_display_the_musical_label() {
    let params = DelayParams::default();
    for &(index, id) in DIVISION_PARAMS {
        let p = params.param_at(index);
        assert_eq!(p.id(), id, "param index {index} is no longer {id}");
        for (i, label) in DIVISION_LABELS.iter().enumerate() {
            assert_eq!(
                p.display(i as f64),
                *label,
                "{id} at index {i} must read as {label}"
            );
        }
    }
}

#[test]
fn the_label_table_covers_the_whole_range() {
    let params = DelayParams::default();
    for &(index, id) in DIVISION_PARAMS {
        let p = params.param_at(index);
        assert_eq!(p.min_plain(), 0.0, "{id} must start at index 0");
        assert_eq!(
            p.max_plain(),
            DIVISION_LABELS.len() as f64 - 1.0,
            "{id}'s range must match the {} labels",
            DIVISION_LABELS.len()
        );
        assert!(p.is_stepped(), "{id} must be stepped");
    }
}

#[test]
fn labels_round_trip_through_parse() {
    let params = DelayParams::default();
    let p = params.param_at(15);
    for (i, label) in DIVISION_LABELS.iter().enumerate() {
        assert_eq!(p.parse(label), Some(i as f64), "parsing {label}");
        assert_eq!(
            p.parse(&label.to_lowercase()),
            Some(i as f64),
            "parsing {label} case-insensitively"
        );
        assert_eq!(p.parse(&format!("  {label} ")), Some(i as f64));
    }
    // A raw index still parses, so automation written against the old
    // integer display keeps working.
    assert_eq!(p.parse("3"), Some(3.0));
    assert_eq!(p.parse("not a division"), None);
}

#[test]
fn out_of_range_display_clamps_to_a_label() {
    let params = DelayParams::default();
    let p = params.param_at(1);
    let last = *DIVISION_LABELS.last().unwrap();
    assert_eq!(p.display(999.0), last);
    assert_eq!(p.display(-4.0), DIVISION_LABELS[0]);
}

#[test]
fn helpers_agree_with_the_table() {
    for (i, label) in DIVISION_LABELS.iter().enumerate() {
        assert_eq!(division_label(i), *label);
        assert_eq!(division_from_label(label), Some(i));
    }
    assert_eq!(division_label(usize::MAX), *DIVISION_LABELS.last().unwrap());
    assert_eq!(division_from_label("1/32"), None);
}

/// The label must reach the host through the same path the CLAP bridge
/// uses: `plugin.param(i).display(value)`.
#[test]
fn the_plugin_surface_reports_labels_to_the_host() {
    let plugin = ResonanceDelay::new();
    for &(index, id) in DIVISION_PARAMS {
        let p = plugin.param(index);
        assert_eq!(p.id(), id);
        assert_eq!(p.display(7.0), "1/8");
    }
}
