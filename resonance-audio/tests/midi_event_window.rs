//! Null test for the per-block MIDI-event window (`mixer::midi_events`).
//!
//! The windowed collector rejects notes with two integer comparisons
//! before running the tempo-map conversion. That is only sound if the
//! rejected notes could never have produced an event, so this test pins
//! the windowed implementation against a straightforward reference that
//! converts *every* note — the algorithm that shipped before the window —
//! over a wide sweep of clip trims, tempo maps, note lengths and
//! playheads. Any divergence (a dropped note, a shifted sample offset, a
//! different ordering) fails here.

use resonance_audio::collect_midi_events_bounce;
use resonance_audio::types::*;

const SR: u32 = 48_000;

/// Reference implementation: the pre-window collector, verbatim.
fn reference_collect(
    midi_clips: &[MidiClip],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    tempo_map: &TempoMap,
    sample_rate: u32,
    out: &mut Vec<(bool, u8, u32)>,
) {
    out.clear();
    let buf_end = playhead + frames as u64;

    for clip in midi_clips.iter().filter(|c| c.track_id == track_id) {
        let visible_start = clip.trim_start_ticks;
        let visible_end = clip.duration_ticks.saturating_sub(clip.trim_end_ticks);
        // The windowed version skips fully-trimmed clips outright; the
        // original would underflow on them, so the reference matches the
        // new guard rather than the old undefined behaviour.
        if visible_end <= visible_start {
            continue;
        }

        for note in &clip.notes {
            if note.start_tick + note.duration_ticks <= visible_start {
                continue;
            }
            if note.start_tick >= visible_end {
                continue;
            }
            let effective_start = note.start_tick.max(visible_start);
            let effective_end = (note.start_tick + note.duration_ticks).min(visible_end);
            let note_abs_start = tempo_map.tick_to_abs_sample(
                clip.start_sample,
                effective_start - visible_start,
                sample_rate,
            );
            let note_abs_end = tempo_map.tick_to_abs_sample(
                clip.start_sample,
                effective_end - visible_start,
                sample_rate,
            );
            if note_abs_start >= playhead && note_abs_start < buf_end {
                out.push((true, note.note, (note_abs_start - playhead) as u32));
            }
            if note_abs_end >= playhead && note_abs_end < buf_end {
                out.push((false, note.note, (note_abs_end - playhead) as u32));
            }
        }
    }
    out.sort_by_key(|e| (e.2, e.0));
}

/// Deterministic xorshift so the sweep is reproducible without a `rand`
/// dependency in this crate's dev-dependencies.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn upto(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

fn flat_map(bpm: f32, num: u8, den: u8) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = bpm;
    tm.numerator = num;
    tm.denominator = den;
    tm.rebuild_bar_table(SR);
    tm
}

fn ramped_map() -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = 90.0;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.tempo_points = vec![
        TempoPoint { bar: 0, bpm: 90.0 },
        TempoPoint { bar: 40, bpm: 174.0 },
        TempoPoint { bar: 41, bpm: 60.0 },
        TempoPoint { bar: 120, bpm: 128.0 },
    ];
    tm.rebuild_bar_table(SR);
    tm
}

fn build_clip(rng: &mut Rng, id: ClipId, track_id: TrackId, notes: usize) -> MidiClip {
    let bars = 8 + rng.upto(120);
    let duration_ticks = bars * 4 * TICKS_PER_QUARTER_NOTE as u64;
    let trim_start = rng.upto(duration_ticks / 4);
    let trim_end = rng.upto(duration_ticks / 4);
    MidiClip {
        id,
        track_id,
        start_sample: rng.upto(SR as u64 * 30),
        duration_ticks,
        notes: (0..notes)
            .map(|_| MidiNote {
                note: 21 + rng.upto(80) as u8,
                velocity: 0.5,
                start_tick: rng.upto(duration_ticks),
                // Durations from a 32nd up to eight bars, so notes that
                // start long before the block still end inside it.
                duration_ticks: 1 + rng.upto(TICKS_PER_QUARTER_NOTE as u64 * 32),
            })
            .collect(),
        name: format!("c{id}"),
        trim_start_ticks: trim_start,
        trim_end_ticks: trim_end,
    }
}

fn sweep(tm: &TempoMap, seed: u64, frames: usize) {
    let mut rng = Rng(seed);
    let clips: Vec<MidiClip> = (0..5)
        .map(|i| build_clip(&mut rng, i, 1, 200))
        .collect();

    let mut got: Vec<PendingNoteEvent> = Vec::new();
    let mut want: Vec<(bool, u8, u32)> = Vec::new();

    // Sweep the whole arrangement block by block, plus a set of random
    // seek positions (a seek lands the playhead anywhere).
    let span = SR as u64 * 400;
    let mut playhead = 0u64;
    let mut checked = 0usize;
    while playhead < span {
        collect_midi_events_bounce(&clips, 1, playhead, frames, tm, SR, &mut got);
        reference_collect(&clips, 1, playhead, frames, tm, SR, &mut want);
        let got_tuples: Vec<(bool, u8, u32)> = got
            .iter()
            .map(|e| (e.is_note_on, e.note, e.sample_offset))
            .collect();
        assert_eq!(
            got_tuples, want,
            "windowed collector diverged at playhead {playhead}"
        );
        checked += 1;
        // Stride by a whole block most of the time, occasionally jump.
        playhead += if rng.upto(64) == 0 {
            rng.upto(SR as u64 * 20)
        } else {
            frames as u64
        };
    }
    assert!(checked > 1000, "sweep should cover many blocks, got {checked}");
}

#[test]
fn windowed_collector_matches_reference_at_120bpm() {
    sweep(&flat_map(120.0, 4, 4), 0x1234_5678_9abc_def0, 128);
}

#[test]
fn windowed_collector_matches_reference_at_odd_meter_and_tempo() {
    sweep(&flat_map(174.3, 7, 8), 0x0f0f_0f0f_dead_beef, 128);
}

#[test]
fn windowed_collector_matches_reference_under_tempo_ramps() {
    sweep(&ramped_map(), 0xfeed_face_cafe_b0ba, 128);
}

#[test]
fn windowed_collector_matches_reference_at_large_buffers() {
    // The bounce path renders in much larger chunks than the live
    // quantum; the window has to hold there too.
    sweep(&flat_map(120.0, 4, 4), 0x00c0_ffee_0000_0001, 1024);
}

#[test]
fn windowed_collector_matches_reference_with_no_bar_table() {
    // A tempo map that was never `rebuild_bar_table`d takes the flat
    // samples-per-tick fallback inside the conversion.
    let mut tm = TempoMap::default();
    tm.bpm = 132.0;
    sweep(&tm, 0x5555_aaaa_5555_aaaa, 128);
}

#[test]
fn windowed_collector_keeps_long_notes_ending_inside_the_block() {
    // Regression guard for the head of the window: a note that started
    // many bars earlier must still emit its NoteOff.
    let tm = flat_map(120.0, 4, 4);
    let spb = SR as u64 / 2; // 0.5 s per beat at 120 BPM
    let clip = MidiClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 256 * 4 * TICKS_PER_QUARTER_NOTE as u64,
        // Enough notes to trip the window (the small-clip bypass keeps
        // clips under WINDOW_MIN_NOTES on the unfiltered path).
        notes: std::iter::once(MidiNote {
            note: 60,
            velocity: 0.9,
            start_tick: 0,
            duration_ticks: 200 * 4 * TICKS_PER_QUARTER_NOTE as u64,
        })
        .chain((0..64).map(|i| MidiNote {
            note: 40,
            velocity: 0.5,
            start_tick: i * TICKS_PER_QUARTER_NOTE as u64,
            duration_ticks: 10,
        }))
        .collect(),
        name: "long".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    };
    let clips = vec![clip];
    let note_off_at = 200 * 4 * spb;
    let mut out = Vec::new();
    let block_start = note_off_at - (note_off_at % 128);
    collect_midi_events_bounce(&clips, 1, block_start, 128, &tm, SR, &mut out);
    assert!(
        out.iter().any(|e| !e.is_note_on && e.note == 60),
        "the eight-hundred-beat note's NoteOff must survive the window; got {out:?}"
    );
}
