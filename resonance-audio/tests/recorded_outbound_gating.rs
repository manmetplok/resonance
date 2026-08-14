//! Integration tests for the Recorded-playback gating of the timeline →
//! hardware MIDI scheduler (`engine/midi/outbound.rs`, doc #257, todo
//! #1099).
//!
//! When an external-instrument track's playback source is `Recorded`, a
//! span covered by a recorded take (an audio clip on the track) must not
//! re-drive the hardware: NoteOns starting inside the span are skipped,
//! and a note held across the span boundary is released the moment the
//! poll window enters the span (no stuck notes on the synth). Outside
//! covered spans the track behaves exactly live, and `Live` mode is
//! behaviour-identical to before the mode existed. The tests drive the
//! pure emission core `emit_outbound_notes` with a capturing fake
//! `OutboundNoteSink` — the same core the engine poll and the realtime
//! bounce drive run — plus `outbound_track_snapshot` for the per-track
//! gating-flag derivation.

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;

use resonance_audio::types::*;
use resonance_audio::{
    emit_outbound_notes, outbound_track_snapshot, OutboundNoteSink, OutboundTrack,
};
use resonance_common::PlaybackSource;

const SR: u32 = 48_000;
const TRACK: TrackId = 1;

/// At the default tempo map (120 BPM, 480 TPQN, 48 kHz) one tick is
/// exactly 50 samples, so tick positions in the fixtures map to round
/// sample positions: tick t → t * 50.
const SAMPLES_PER_TICK: u64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Msg {
    On(TrackId, u8, u8, u8),
    Off(TrackId, u8, u8),
}

#[derive(Default)]
struct CaptureSink(Vec<Msg>);

impl OutboundNoteSink for CaptureSink {
    fn note_on(&mut self, track_id: TrackId, channel: u8, note: u8, velocity: u8) {
        self.0.push(Msg::On(track_id, channel, note, velocity));
    }
    fn note_off(&mut self, track_id: TrackId, channel: u8, note: u8) {
        self.0.push(Msg::Off(track_id, channel, note));
    }
}

fn midi_clip(notes: Vec<MidiNote>) -> MidiClip {
    MidiClip {
        id: 1,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 4_000,
        notes,
        name: "part".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn note(pitch: u8, start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 1.0,
        start_tick,
        duration_ticks,
    }
}

/// A recorded take on `TRACK` spanning `[start, start + frames)` on the
/// timeline.
fn take(start: u64, frames: usize) -> AudioClip {
    AudioClip {
        id: 100,
        track_id: TRACK,
        start_sample: start,
        source: ClipSource::Memory(vec![0.0; frames * 2]),
        name: "take".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

fn track_entry(gate_recorded: bool) -> OutboundTrack {
    OutboundTrack {
        track_id: TRACK,
        channel: 0,
        gate_recorded,
    }
}

/// Run the emitter over contiguous poll windows of `step` samples across
/// `[0, end)`, mimicking the engine poll cadence, and return the emitted
/// message stream.
fn run_windows(
    tracks: &[OutboundTrack],
    midi_clips: &[MidiClip],
    audio_clips: &[AudioClip],
    end: u64,
    step: u64,
    held: &mut HashMap<(TrackId, u8), (u64, u8)>,
) -> Vec<Msg> {
    let tempo = TempoMap::default();
    let mut sink = CaptureSink::default();
    let mut last = 0u64;
    while last < end {
        let curr = (last + step).min(end);
        emit_outbound_notes(
            tracks, midi_clips, audio_clips, &tempo, SR, last, curr, held, &mut sink,
        );
        last = curr;
    }
    sink.0
}

// -- Live parity ---------------------------------------------------------

/// `Live` mode (`gate_recorded: false`) is behaviour-identical to today
/// even when takes exist on the track: every note fires and releases.
#[test]
fn live_mode_ignores_takes() {
    let clips = vec![midi_clip(vec![
        note(60, 0, 40),    // samples [0, 2_000)
        note(62, 100, 40),  // samples [5_000, 7_000) — inside the take
        note(64, 300, 20),  // samples [15_000, 16_000)
    ])];
    let takes = vec![take(4_000, 8_000)]; // covers [4_000, 12_000)
    let mut held = HashMap::new();

    let with_takes = run_windows(
        &[track_entry(false)],
        &clips,
        &takes,
        20_000,
        1_000,
        &mut held,
    );
    held.clear();
    let without_takes =
        run_windows(&[track_entry(false)], &clips, &[], 20_000, 1_000, &mut held);

    assert_eq!(with_takes, without_takes);
    assert_eq!(
        with_takes,
        vec![
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 62, 127),
            Msg::Off(TRACK, 0, 62),
            Msg::On(TRACK, 0, 64, 127),
            Msg::Off(TRACK, 0, 64),
        ]
    );
}

// -- Covered-span gating -------------------------------------------------

/// With the take covering `[4_000, 12_000)` and mode `Recorded`, the note
/// starting inside the span never reaches the hardware, while the notes
/// before and after the span (the live fallback) still fire and release.
#[test]
fn covered_note_start_is_skipped_uncovered_spans_stay_live() {
    let clips = vec![midi_clip(vec![
        note(60, 0, 40),    // [0, 2_000) — before the take
        note(62, 100, 40),  // [5_000, 7_000) — covered ⇒ skipped
        note(64, 300, 20),  // [15_000, 16_000) — after the take
    ])];
    let takes = vec![take(4_000, 8_000)];
    let mut held = HashMap::new();

    let stream = run_windows(
        &[track_entry(true)],
        &clips,
        &takes,
        20_000,
        1_000,
        &mut held,
    );

    assert_eq!(
        stream,
        vec![
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 64, 127),
            Msg::Off(TRACK, 0, 64),
        ]
    );
    assert!(held.is_empty());
}

/// A note held across the span boundary gets its NoteOff the moment the
/// poll window enters the covered span — not at the note's timeline end
/// (which lies inside the span), and it is never left hanging.
#[test]
fn held_note_released_when_playhead_enters_span() {
    // Note 60: [0, 10_000) — its end sits inside the take [4_000, 12_000).
    let clips = vec![midi_clip(vec![note(60, 0, 200)])];
    let takes = vec![take(4_000, 8_000)];
    let tempo = TempoMap::default();
    let tracks = [track_entry(true)];
    let mut held = HashMap::new();
    let mut sink = CaptureSink::default();

    // [0, 1_000): the note fires and is held.
    emit_outbound_notes(
        &tracks, &clips, &takes, &tempo, SR, 0, 1_000, &mut held, &mut sink,
    );
    assert_eq!(sink.0, vec![Msg::On(TRACK, 0, 60, 127)]);
    assert!(held.contains_key(&(TRACK, 60)));

    // [3_000, 4_000): still entirely before the span — nothing released.
    emit_outbound_notes(
        &tracks, &clips, &takes, &tempo, SR, 3_000, 4_000, &mut held, &mut sink,
    );
    assert_eq!(sink.0.len(), 1);
    assert!(held.contains_key(&(TRACK, 60)));

    // [4_000, 5_000): the window enters the covered span — NoteOff now.
    emit_outbound_notes(
        &tracks, &clips, &takes, &tempo, SR, 4_000, 5_000, &mut held, &mut sink,
    );
    assert_eq!(
        sink.0,
        vec![Msg::On(TRACK, 0, 60, 127), Msg::Off(TRACK, 0, 60)]
    );
    assert!(held.is_empty());

    // [9_000, 11_000): crossing the note's timeline end emits nothing
    // more — the note was already released at span entry.
    emit_outbound_notes(
        &tracks, &clips, &takes, &tempo, SR, 9_000, 11_000, &mut held, &mut sink,
    );
    assert_eq!(sink.0.len(), 2);
}

/// The span-entry drain runs before the window's NoteOns, so a note that
/// starts in the same window but *after* the span's end still fires and
/// stays held — leaving a span must not swallow (or stale-release) the
/// next live note.
#[test]
fn note_after_span_end_survives_entry_drain_in_same_window() {
    let clips = vec![midi_clip(vec![
        note(57, 60, 340),  // [3_000, 20_000) — held into the span
        note(59, 250, 10),  // [12_500, 13_000) — right after the span end
    ])];
    let takes = vec![take(4_000, 8_000)]; // [4_000, 12_000)
    let tempo = TempoMap::default();
    let tracks = [track_entry(true)];
    let mut held = HashMap::new();
    let mut sink = CaptureSink::default();

    emit_outbound_notes(
        &tracks, &clips, &takes, &tempo, SR, 3_000, 3_100, &mut held, &mut sink,
    );
    // One window sweeping from inside the span past its end to 13_000:
    // the held note is drained first, then the post-span note fires.
    emit_outbound_notes(
        &tracks, &clips, &takes, &tempo, SR, 11_000, 13_000, &mut held, &mut sink,
    );

    assert_eq!(
        sink.0,
        vec![
            Msg::On(TRACK, 0, 57, 127),
            Msg::Off(TRACK, 0, 57),
            Msg::On(TRACK, 0, 59, 127),
        ]
    );
    // 59 ends at exactly 13_000 — window end is exclusive, so it is
    // still held for the next poll (normal live discipline).
    assert_eq!(held.len(), 1);
    assert!(held.contains_key(&(TRACK, 59)));
}

/// One large forward window sweeping clean across the whole span (a
/// sub-second jump classified as `Continue`) emits nothing for a note
/// whose start lies inside the span — no stale NoteOns on exit.
#[test]
fn sweeping_past_span_emits_no_stale_noteon() {
    let clips = vec![midi_clip(vec![note(62, 100, 40)])]; // [5_000, 7_000)
    let takes = vec![take(4_000, 8_000)];
    let mut held = HashMap::new();

    let stream = run_windows(
        &[track_entry(true)],
        &clips,
        &takes,
        13_000,
        13_000, // single window [0, 13_000)
        &mut held,
    );

    assert_eq!(stream, Vec::<Msg>::new());
    assert!(held.is_empty());
}

/// Loop wrapping across the span boundary: approach the span, wrap from
/// inside the loop back to `loop_in`, and replay. Gating holds on every
/// pass — the pre-span note fires each iteration, the covered note
/// never does, and no note is left hanging at the seam (the held map is
/// empty at wrap time because span entry already released it).
#[test]
fn loop_wrap_across_span_boundary_replays_uncovered_notes_only() {
    let clips = vec![midi_clip(vec![
        note(60, 0, 40),   // [0, 2_000) — before the span
        note(62, 100, 40), // [5_000, 7_000) — covered
    ])];
    let takes = vec![take(4_000, 8_000)];
    let tempo = TempoMap::default();
    let tracks = [track_entry(true)];
    let mut held = HashMap::new();
    let mut sink = CaptureSink::default();

    // First loop pass: [0, 8_000) in 1_000-sample polls.
    let mut last = 0u64;
    while last < 8_000 {
        emit_outbound_notes(
            &tracks,
            &clips,
            &takes,
            &tempo,
            SR,
            last,
            last + 1_000,
            &mut held,
            &mut sink,
        );
        last += 1_000;
    }
    // Loop wrap: the poll's discontinuity handling drains all held
    // notes before re-emitting from loop_in (existing behaviour —
    // `outbound_step_start` classification). Nothing is held here, so
    // the wrap is silent.
    assert!(held.is_empty());

    // Second pass starts at loop_in = 0: the pre-span note fires again.
    emit_outbound_notes(
        &tracks, &clips, &takes, &tempo, SR, 0, 1_000, &mut held, &mut sink,
    );

    assert_eq!(
        sink.0,
        vec![
            Msg::On(TRACK, 0, 60, 127),
            Msg::Off(TRACK, 0, 60),
            Msg::On(TRACK, 0, 60, 127),
        ]
    );
}

/// Gating is per track: a second, ungated track keeps emitting over the
/// gated track's covered span.
#[test]
fn gating_is_scoped_to_the_recorded_track() {
    const OTHER: TrackId = 2;
    let mut other_clip = midi_clip(vec![note(62, 100, 40)]); // [5_000, 7_000)
    other_clip.id = 2;
    other_clip.track_id = OTHER;
    let clips = vec![midi_clip(vec![note(62, 100, 40)]), other_clip];
    let takes = vec![take(4_000, 8_000)]; // take on TRACK only
    let tracks = [
        track_entry(true),
        OutboundTrack {
            track_id: OTHER,
            channel: 5,
            gate_recorded: false,
        },
    ];
    let mut held = HashMap::new();

    let stream = run_windows(&tracks, &clips, &takes, 8_000, 1_000, &mut held);

    assert_eq!(
        stream,
        vec![Msg::On(OTHER, 5, 62, 127), Msg::Off(OTHER, 5, 62)]
    );
}

// -- Snapshot / gating-flag derivation -----------------------------------

fn output_track(id: TrackId) -> Track {
    let mut t = Track::new(id, format!("ext {id}"));
    t.midi_output_device.store(Some(Arc::new("dev".into())));
    t.midi_output_channel = Some(3);
    t
}

#[test]
fn snapshot_gates_recorded_unarmed_tracks_only() {
    let mut tracks = IndexMap::new();

    let live = output_track(1); // default playback source: Live
    let recorded = output_track(2);
    recorded.set_playback_source(PlaybackSource::Recorded);
    let armed = output_track(3);
    armed.set_playback_source(PlaybackSource::Recorded);
    armed.set_record_armed(true); // punch-in: stays fully live

    tracks.insert(1, live);
    tracks.insert(2, recorded);
    tracks.insert(3, armed);

    let snap = outbound_track_snapshot(&tracks);
    assert_eq!(
        snap,
        vec![
            OutboundTrack {
                track_id: 1,
                channel: 3,
                gate_recorded: false
            },
            OutboundTrack {
                track_id: 2,
                channel: 3,
                gate_recorded: true
            },
            OutboundTrack {
                track_id: 3,
                channel: 3,
                gate_recorded: false
            },
        ]
    );
}

#[test]
fn snapshot_skips_muted_and_deviceless_tracks() {
    let mut tracks = IndexMap::new();

    let muted = output_track(1);
    muted.set_muted(true);
    tracks.insert(1, muted);
    // No MIDI output device configured — not an outbound track at all.
    tracks.insert(2, Track::new(2, "plain".into()));

    assert!(outbound_track_snapshot(&tracks).is_empty());
}
