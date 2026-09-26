//! ARCH-04 D-3: the app allocates every bus id now
//! (`TrackRegistry::allocate_bus_id`), and `AudioCommand::AddBus` carries
//! a concrete `id` rather than an optional hint. The engine has no
//! allocator of its own left for busses — `next_bus_id` and the
//! hint-vs-`RETURN_BUS_ID_BASE` rule are gone, same shape as ARCH-04 D-1's
//! plugin ids (`tests/clap_host/plugin_id_duplicate_rejected.rs`).
//!
//! What the engine still owes the app is refusing a collision instead of
//! silently replacing the live bus: two adds for the same id would
//! otherwise leave one entry in `ctx.busses` that the second add's caller
//! believes is a bus of its own. Drives the real `handle_add_bus` handler
//! via `EngineHandlerHarness`, so what's proven is the actual `ctx.busses`
//! map and the actual `AudioEvent::Error` — not a description of the rule.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioEvent, BusId, EngineErrorKind};

fn error_kind(events: &[AudioEvent]) -> Option<EngineErrorKind> {
    events.iter().find_map(|e| match e {
        AudioEvent::Error(err) => Some(err.kind),
        _ => None,
    })
}

fn bus_ids(harness: &EngineHandlerHarness) -> Vec<BusId> {
    harness.test_bus_ids()
}

#[test]
fn a_duplicate_id_is_refused_and_does_not_replace_the_live_bus() {
    let mut harness = EngineHandlerHarness::new();

    let first = harness.add_bus(1, Some("Reverb".to_string()));
    assert!(
        first
            .iter()
            .any(|e| matches!(e, AudioEvent::BusAdded { bus_id: 1, .. })),
        "events: {first:?}"
    );
    assert_eq!(bus_ids(&harness), vec![1]);

    // A second add asking for the SAME id is refused — not re-numbered,
    // not silently accepted as a rename of the first bus.
    let second = harness.add_bus(1, Some("Delay".to_string()));
    assert_eq!(
        error_kind(&second),
        Some(EngineErrorKind::Internal),
        "a duplicate id is a caller invariant violation, not a transient \
         Busy condition — events: {second:?}"
    );
    assert!(
        !second
            .iter()
            .any(|e| matches!(e, AudioEvent::BusAdded { .. })),
        "the refused add must not also emit a BusAdded: {second:?}"
    );

    // The original bus is untouched: still the only entry, and still
    // named "Reverb" — a silent replace would leave the count at 1 too,
    // but with the name overwritten.
    assert_eq!(bus_ids(&harness), vec![1], "no second bus was added");
    assert_eq!(
        harness.test_bus_name(1).as_deref(),
        Some("Reverb"),
        "the refused add must not rename the live bus"
    );
}
