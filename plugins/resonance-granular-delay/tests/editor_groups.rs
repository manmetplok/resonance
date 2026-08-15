//! Editor control-surface data tests (ba todo #1079): the §9 group
//! table covers the full declared parameter surface exactly once, the
//! widget mapping is consistent with each parameter's declared range,
//! and every cached static option list matches its parameter and its
//! source of truth.

#![cfg(feature = "editor")]

use resonance_granular_delay::choice::ChoiceParam;
use resonance_granular_delay::dsp::{
    DampingFilter, FbRoute, QualityTier, Scheduler, TimeMode,
};
use resonance_granular_delay::editor::controls::{
    control_kind, ControlKind, FB_ROUTE_LABELS, FILTER_TYPE_LABELS, GROUPS, GROUP_ROWS,
    QUALITY_LABELS, QUANTIZE_LABELS, ROOT_LABELS, SCALE_LABELS, SCHEDULER_LABELS,
    TIME_MODE_LABELS,
};
use resonance_granular_delay::params::{GranularDelayParams, PARAM_COUNT};
use resonance_granular_delay::quantize::PitchQuantize;
use resonance_music_theory::Mode;

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
    // sync, fb_pitch, density_sync, freeze, align
    let expected = [0usize, 6, 9, 19, 29];
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

// --- Choice params: one definition, three consumers (ba todo #1267) ---

/// Parameter index of each mode choice (see `param_at`), with the enum
/// table the editor and the audio path both resolve through.
const CHOICE_PARAMS: &[(usize, &str, &[&str], usize)] = &[
    (3, "time_mode", TIME_MODE_LABELS, TimeMode::ALL.len()),
    (5, "fb_route", FB_ROUTE_LABELS, FbRoute::ALL.len()),
    (10, "scheduler", SCHEDULER_LABELS, Scheduler::ALL.len()),
    (12, "pitch_quantize", QUANTIZE_LABELS, PitchQuantize::ALL.len()),
    (20, "filter_type", FILTER_TYPE_LABELS, DampingFilter::ALL.len()),
    (26, "quality", QUALITY_LABELS, QualityTier::ALL.len()),
];

/// Every mode choice: the declared parameter range, the enum's variant
/// count and the editor's label list must agree, per parameter.
#[test]
fn choice_param_ranges_match_their_enum_tables() {
    let params = GranularDelayParams::default();
    for &(index, id, labels, variants) in CHOICE_PARAMS {
        let p = params.param_at(index);
        assert_eq!(p.id(), id, "param index {index} is not {id}");
        assert_eq!(p.min_plain(), 0.0, "choice param {id} must start at 0");
        assert_eq!(
            p.max_plain() + 1.0,
            variants as f64,
            "{id} declares range 0..={} but its enum has {variants} variants",
            p.max_plain()
        );
        assert_eq!(
            labels.len(),
            variants,
            "{id} has {} labels for {variants} variants",
            labels.len()
        );
        let mut seen = std::collections::HashSet::new();
        for label in labels {
            assert!(seen.insert(*label), "{id} repeats the label {label}");
        }
    }
}

/// `from_index` covers every in-range value with no catch-all: each
/// index maps to a distinct variant that reports the same index back,
/// and out-of-range values are rejected by `try_from_index` (the audio
/// path's total `from_index` resolves them to the documented fallback).
fn assert_round_trip<T>(id: &str)
where
    T: ChoiceParam + std::fmt::Debug,
{
    for (i, expected) in T::ALL.iter().enumerate() {
        let index = i as i32;
        assert_eq!(
            T::try_from_index(index),
            Some(*expected),
            "{id}: index {index} does not map to {expected:?}"
        );
        assert_eq!(
            T::from_index(index).index(),
            i,
            "{id}: variant {expected:?} does not report index {i}"
        );
        assert_eq!(
            T::from_index(index).label(),
            T::LABELS[i],
            "{id}: label of index {i} is out of sync"
        );
    }
    for out_of_range in [-1, T::ALL.len() as i32, 1_000] {
        assert!(
            T::try_from_index(out_of_range).is_none(),
            "{id}: index {out_of_range} is outside the declared range but resolved"
        );
        assert_eq!(
            T::from_index(out_of_range),
            T::FALLBACK,
            "{id}: out-of-range index {out_of_range} must resolve to the fallback"
        );
    }
}

#[test]
fn choice_enums_round_trip_every_in_range_index() {
    assert_round_trip::<TimeMode>("time_mode");
    assert_round_trip::<FbRoute>("fb_route");
    assert_round_trip::<Scheduler>("scheduler");
    assert_round_trip::<PitchQuantize>("pitch_quantize");
    assert_round_trip::<DampingFilter>("filter_type");
    assert_round_trip::<QualityTier>("quality");
}

/// The integer → mode mapping the plugin has always had (the arms that
/// used to live inline in `lib.rs::process`), pinned so a reordered
/// enum cannot silently repurpose a saved parameter value.
#[test]
fn choice_indices_keep_their_historical_meaning() {
    assert_eq!(TimeMode::from_index(0), TimeMode::Fade);
    assert_eq!(TimeMode::from_index(1), TimeMode::Repitch);
    assert_eq!(TimeMode::from_index(2), TimeMode::PerGrain);
    assert_eq!(TimeMode::from_index(7), TimeMode::PerGrain);

    assert_eq!(FbRoute::from_index(0), FbRoute::WetToBuffer);
    assert_eq!(FbRoute::from_index(1), FbRoute::OutputOnly);
    assert_eq!(FbRoute::from_index(2), FbRoute::PingPong);
    assert_eq!(FbRoute::from_index(7), FbRoute::OutputOnly);

    assert_eq!(QualityTier::from_index(0), QualityTier::LoFi);
    assert_eq!(QualityTier::from_index(1), QualityTier::Normal);
    assert_eq!(QualityTier::from_index(2), QualityTier::Hq);
    assert_eq!(QualityTier::from_index(7), QualityTier::Normal);

    assert_eq!(PitchQuantize::from_index(0), PitchQuantize::Off);
    assert_eq!(PitchQuantize::from_index(1), PitchQuantize::Semitones);
    assert_eq!(PitchQuantize::from_index(2), PitchQuantize::Scale);
    assert_eq!(PitchQuantize::from_index(7), PitchQuantize::Off);

    assert!(!DampingFilter::from_index(0).is_highpass());
    assert!(DampingFilter::from_index(1).is_highpass());
    assert!(!DampingFilter::from_index(7).is_highpass());

    // Scheduler resolves to both an engine mode and the pitch-sync flag
    // (index 2 = Voice runs Async grains under the PSOLA bus).
    assert_eq!(
        Scheduler::from_index(0).engine_mode(),
        resonance_dsp::SchedulerMode::Sync
    );
    for index in [1, 2, 7] {
        assert_eq!(
            Scheduler::from_index(index).engine_mode(),
            resonance_dsp::SchedulerMode::Async,
            "scheduler index {index} must run the async cloud"
        );
    }
    assert!(Scheduler::from_index(2).pitch_sync());
    for index in [0, 1, 7] {
        assert!(
            !Scheduler::from_index(index).pitch_sync(),
            "scheduler index {index} must not engage the voice path"
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

