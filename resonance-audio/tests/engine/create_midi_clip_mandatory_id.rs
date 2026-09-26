//! D-7c: `AudioCommand::CreateMidiClip` takes a mandatory `clip_id` — the
//! app allocates it (`ComposeState::fresh_derived_clip_id`, the general
//! clip allocator) and the engine no longer draws one from `next_clip_id`
//! for this command. What the engine still owes the app is refusing a
//! collision instead of silently replacing the live clip — the same shape
//! as `bus_id_duplicate_rejected.rs` (ARCH-04 D-3) and
//! `clap_host/plugin_id_duplicate_rejected.rs` (D-1). Drives the real
//! `handle_create_midi_clip` via `EngineHandlerHarness`, so what's proven
//! is the actual `ctx.midi_clips` list and the actual `AudioEvent::Error`
//! — not a description of the rule.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioClip, AudioEvent, ClipId, ClipSource, EngineErrorKind, FadeCurve, TrackId};

const TRACK: TrackId = 3;

/// Minimal in-memory audio clip, for the shared-id-space collision test —
/// no WAV file needed since it's never loaded off the import worker.
fn audio_clip(id: ClipId) -> AudioClip {
    AudioClip {
        id,
        track_id: TRACK,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.0; 16]),
        name: "audio".to_string(),
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

fn error_kind(events: &[AudioEvent]) -> Option<EngineErrorKind> {
    events.iter().find_map(|e| match e {
        AudioEvent::Error(err) => Some(err.kind),
        _ => None,
    })
}

#[test]
fn create_midi_clip_honours_the_given_id() {
    let mut h = EngineHandlerHarness::new();
    const CLIP_ID: ClipId = 1 << 40; // a realistic app-derived id
    let events = h.create_midi_clip(CLIP_ID, TRACK, 0, 1920);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::MidiClipCreated { clip_id, .. } if *clip_id == CLIP_ID)),
        "events: {events:?}"
    );
    assert_eq!(h.midi_clip_ids(), vec![CLIP_ID]);
}

#[test]
fn a_duplicate_id_is_refused_and_does_not_disturb_the_live_clip() {
    let mut h = EngineHandlerHarness::new();
    let first = h.create_midi_clip(1, TRACK, 0, 1920);
    assert!(
        first
            .iter()
            .any(|e| matches!(e, AudioEvent::MidiClipCreated { clip_id: 1, .. })),
        "events: {first:?}"
    );
    assert_eq!(h.midi_clip_ids(), vec![1]);

    // A second create asking for the SAME id is refused — not re-numbered,
    // not silently accepted as a replacement of the first clip.
    let second = h.create_midi_clip(1, TRACK, 480, 960);
    assert_eq!(
        error_kind(&second),
        Some(EngineErrorKind::Internal),
        "a duplicate id is a caller invariant violation, not a transient \
         Busy condition — events: {second:?}"
    );
    assert!(
        !second.iter().any(|e| matches!(e, AudioEvent::MidiClipCreated { .. })),
        "the refused create must not also emit a MidiClipCreated: {second:?}"
    );

    // The original clip is untouched: still the only entry, and still
    // named "drawn" — a silent replace would leave the count at 1 too,
    // but with the start/duration overwritten.
    assert_eq!(h.midi_clip_ids(), vec![1], "no second clip was added");
    assert_eq!(
        h.midi_clip_name(1).as_deref(),
        Some("drawn"),
        "the refused create must not rename/replace the live clip"
    );
}

/// The shared clip-id space (D-6 §3): a `CreateMidiClip` must also refuse
/// to collide with an *audio* clip's id, not only another MIDI clip's.
#[test]
fn a_duplicate_id_against_an_audio_clip_is_also_refused() {
    let mut h = EngineHandlerHarness::new();
    h.push_clip(audio_clip(5));

    let events = h.create_midi_clip(5, TRACK, 0, 1920);
    assert_eq!(
        error_kind(&events),
        Some(EngineErrorKind::Internal),
        "events: {events:?}"
    );
    assert!(h.midi_clip_ids().is_empty(), "the refused create must not add a MIDI clip");
}
