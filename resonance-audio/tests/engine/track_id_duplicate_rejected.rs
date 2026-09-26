//! ARCH-04 D-4: the app allocates every track id now
//! (`Resonance::allocate_track_id`), and `AudioCommand::AddTrack` /
//! `AddInstrumentTrack` / `AddVocalTrack` carry a concrete `id` rather than
//! an optional hint. The engine has no allocator of its own left for
//! tracks — `next_track_id` and the hint-vs-`SUB_TRACK_ID_BASE` rule are
//! gone, same shape as ARCH-04 D-1's plugin ids
//! (`tests/clap_host/plugin_id_duplicate_rejected.rs`) and D-3's bus ids
//! (`bus_id_duplicate_rejected.rs`).
//!
//! What the engine still owes the app is refusing a collision instead of
//! silently replacing the live track: two adds for the same id would
//! otherwise leave one entry in `ctx.tracks` that the second add's caller
//! believes is a track of its own — and disturb whatever the first add's
//! caller was already doing with it. Drives the real `handle_add_track`
//! handler via `EngineHandlerHarness`, so what's proven is the actual
//! `ctx.tracks` map and the actual `AudioEvent::Error` — not a description
//! of the rule.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioEvent, EngineErrorKind, TrackId};

fn error_kind(events: &[AudioEvent]) -> Option<EngineErrorKind> {
    events.iter().find_map(|e| match e {
        AudioEvent::Error(err) => Some(err.kind),
        _ => None,
    })
}

fn track_ids(harness: &EngineHandlerHarness) -> Vec<TrackId> {
    harness.test_track_ids()
}

#[test]
fn a_duplicate_id_is_refused_and_does_not_replace_or_disturb_the_live_track() {
    let mut harness = EngineHandlerHarness::new();

    let first = harness.add_track(1, Some("Drums".to_string()));
    assert!(
        first
            .iter()
            .any(|e| matches!(e, AudioEvent::TrackAdded { track_id: 1 })),
        "events: {first:?}"
    );
    assert_eq!(track_ids(&harness), vec![1]);

    // A second add asking for the SAME id is refused — not re-numbered,
    // not silently accepted as a rename of the first track.
    let second = harness.add_track(1, Some("Bass".to_string()));
    assert_eq!(
        error_kind(&second),
        Some(EngineErrorKind::Internal),
        "a duplicate id is a caller invariant violation, not a transient \
         Busy condition — events: {second:?}"
    );
    assert!(
        !second
            .iter()
            .any(|e| matches!(e, AudioEvent::TrackAdded { .. })),
        "the refused add must not also emit a TrackAdded: {second:?}"
    );

    // The original (live) track is untouched: still the only entry, and
    // still named "Drums" — a silent replace would leave the count at 1
    // too, but with the name overwritten, and would have dropped whatever
    // state the live track already carried.
    assert_eq!(track_ids(&harness), vec![1], "no second track was added");
    assert_eq!(
        harness.test_track_name(1).as_deref(),
        Some("Drums"),
        "the refused add must not rename the live track"
    );
}
