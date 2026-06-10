//! Regression test: the bounce render range must convert MIDI clip
//! ends through the tempo map (`TempoMap::tick_to_abs_sample`), not a
//! flat `samples_per_beat` factor. The renderer schedules notes
//! tempo-aware, so under a tempo change a flat conversion mis-sizes
//! the render (truncated when the project slows down, padded when it
//! speeds up).

use std::sync::Arc;

use parking_lot::RwLock;

use resonance_audio::__test_support::{midi_render_range, SharedState};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const TAIL_SAMPLES: u64 = SR as u64 * 2;

fn make_tempo_map(tempo: &[(u32, f32)]) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.tempo_points = tempo
        .iter()
        .map(|&(bar, bpm)| TempoPoint { bar, bpm })
        .collect();
    if let Some(first) = tm.tempo_points.first() {
        tm.bpm = first.bpm;
    }
    tm.rebuild_bar_table(SR);
    tm
}

fn make_clip(track_id: TrackId, start_sample: u64, duration_ticks: u64) -> MidiClip {
    MidiClip {
        id: 1,
        track_id,
        start_sample,
        duration_ticks,
        notes: Vec::new(),
        name: "clip".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn render_range(
    clip: MidiClip,
    tm: &TempoMap,
) -> Result<(SamplePos, SamplePos), &'static str> {
    let shared = Arc::new(SharedState::default());
    let midi_clips = Arc::new(RwLock::new(vec![clip]));
    let tempo_map = Arc::new(arc_swap::ArcSwap::from_pointee(tm.clone()));
    midi_render_range(&midi_clips, &tempo_map, &shared, 1, SR)
}

#[test]
fn range_end_under_tempo_change_matches_tick_to_abs_sample() {
    // 120 BPM for two bars, then a step down to 60 BPM. A clip
    // spanning four 4/4 bars from sample 0 ends much later than the
    // flat 120 BPM conversion predicts.
    let tm = make_tempo_map(&[(0, 120.0), (2, 60.0)]);
    let duration_ticks = 16 * TICKS_PER_QUARTER_NOTE as u64;
    let clip = make_clip(1, 0, duration_ticks);

    let expected_end = tm.tick_to_abs_sample(0, duration_ticks, SR);
    let flat_spt = tm.samples_per_beat(SR) / TICKS_PER_QUARTER_NOTE as f64;
    let flat_end = (duration_ticks as f64 * flat_spt) as u64;
    assert!(
        expected_end > flat_end,
        "test premise: tempo-aware end ({expected_end}) must differ from flat end ({flat_end})"
    );

    let (start, end) = render_range(clip, &tm).expect("clip on source track");
    assert_eq!(start, 0);
    assert_eq!(end, expected_end + TAIL_SAMPLES);
}

#[test]
fn range_end_constant_tempo_matches_flat_conversion() {
    // With a single tempo point the tempo-aware conversion degenerates
    // to the flat one — no behavior change for tempo-less projects.
    let tm = make_tempo_map(&[(0, 120.0)]);
    let duration_ticks = 8 * TICKS_PER_QUARTER_NOTE as u64;
    let start_sample = 10_000;
    let clip = make_clip(1, start_sample, duration_ticks);

    let expected_end = tm.tick_to_abs_sample(start_sample, duration_ticks, SR);
    let flat_spt = tm.samples_per_beat(SR) / TICKS_PER_QUARTER_NOTE as f64;
    let flat_end = start_sample + (duration_ticks as f64 * flat_spt) as u64;
    let diff = expected_end.abs_diff(flat_end);
    assert!(diff <= 1, "constant tempo should match flat (diff {diff})");

    let (start, end) = render_range(clip, &tm).expect("clip on source track");
    assert_eq!(start, start_sample);
    assert_eq!(end, expected_end + TAIL_SAMPLES);
}
