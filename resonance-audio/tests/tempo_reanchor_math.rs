//! Re-anchoring the timeline across a tempo change (ba doc #275 P1.4).
//!
//! The app keeps musical positions across a tempo edit by reading every
//! absolute frame position as a tick under the OLD map and converting it
//! back under the NEW one (`update::tempo_reanchor`). That is only sound
//! if `sample_to_abs_tick` and `tick_to_abs_sample` are inverses across
//! two different maps — which is exactly what these pin, without needing
//! a running app.
//!
//! Before this, absolute positions simply stayed put: a MIDI clip at bar
//! 9 kept its frame offset while the grid moved under it, so it played at
//! bar 10.33 after 120 → 140 while its section placement still said 9.

use resonance_audio::types::{TempoMap, TempoPoint};

const SR: u32 = 48_000;

fn map_at(bpm: f32) -> TempoMap {
    let mut map = TempoMap::default();
    map.bpm = bpm;
    // The bar table is built from the tempo EVENTS, exactly as the app's
    // `rebuild_tempo_map` fills them in from `tempo_events`.
    map.tempo_points = vec![TempoPoint { bar: 0, bpm }];
    map.rebuild_bar_table(SR);
    map
}

/// The whole re-anchor, in miniature: bar 9 at 120 BPM must land on bar 9
/// at 140 BPM, not on the frame it used to occupy.
#[test]
fn a_position_keeps_its_bar_across_a_tempo_change() {
    let old = map_at(120.0);
    let new = map_at(140.0);

    let was = old.bar_to_sample(9);
    let tick = old.sample_to_abs_tick(was, SR);
    let now = new.tick_to_abs_sample(0, tick, SR);

    let want = new.bar_to_sample(9);
    assert!(
        (now as i64 - want as i64).abs() <= 1,
        "bar 9 should re-anchor to {want} at 140 BPM, got {now}"
    );
    assert!(
        now < was,
        "a faster tempo must pull the bar earlier: {was} -> {now}"
    );
}

/// Every bar, not just the one the bug was noticed on.
#[test]
fn every_bar_re_anchors() {
    let old = map_at(120.0);
    let new = map_at(140.0);
    for bar in 0..64u32 {
        let tick = old.sample_to_abs_tick(old.bar_to_sample(bar), SR);
        let got = new.tick_to_abs_sample(0, tick, SR);
        let want = new.bar_to_sample(bar);
        assert!(
            (got as i64 - want as i64).abs() <= 1,
            "bar {bar}: expected {want}, got {got}"
        );
    }
}

/// Slowing down works the same way — the old code's asymmetry (lengths
/// scaled, starts didn't) was direction-independent.
#[test]
fn re_anchoring_works_downwards_too() {
    let old = map_at(140.0);
    let new = map_at(90.0);
    let tick = old.sample_to_abs_tick(old.bar_to_sample(17), SR);
    let got = new.tick_to_abs_sample(0, tick, SR);
    let want = new.bar_to_sample(17);
    assert!(
        (got as i64 - want as i64).abs() <= 1,
        "bar 17 at 90 BPM: expected {want}, got {got}"
    );
    assert!(got > old.bar_to_sample(17), "a slower tempo pushes bars later");
}

/// An off-grid position (a hand-nudged clip, a recorded take that starts
/// a 16th late) keeps its musical offset rather than snapping.
#[test]
fn an_off_grid_position_keeps_its_offset() {
    let old = map_at(120.0);
    let new = map_at(140.0);

    let bar_9 = old.bar_to_sample(9);
    let bar_10 = old.bar_to_sample(10);
    let quarter_in = bar_9 + (bar_10 - bar_9) / 4;

    let tick = old.sample_to_abs_tick(quarter_in, SR);
    let got = new.tick_to_abs_sample(0, tick, SR);

    let new_9 = new.bar_to_sample(9);
    let new_10 = new.bar_to_sample(10);
    let want = new_9 + (new_10 - new_9) / 4;
    assert!(
        (got as i64 - want as i64).abs() <= 2,
        "a quarter into bar 9 should stay a quarter into bar 9: expected {want}, got {got}"
    );
}

/// Re-anchoring to the same tempo is the identity — a no-op tempo commit
/// must not drift the arrangement one frame at a time.
#[test]
fn an_unchanged_tempo_moves_nothing() {
    let map = map_at(128.0);
    for bar in 0..32u32 {
        let at = map.bar_to_sample(bar);
        let tick = map.sample_to_abs_tick(at, SR);
        assert_eq!(
            map.tick_to_abs_sample(0, tick, SR),
            at,
            "bar {bar} round-trips exactly under one map"
        );
    }
}
