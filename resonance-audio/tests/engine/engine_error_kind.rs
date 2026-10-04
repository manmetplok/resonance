//! Guard for C-1 (`refactor-intent.md` Epic C): `AudioEvent::Error` now
//! carries an `EngineError { kind, message }` instead of a bare `String`,
//! and the ~39 emit sites were classified in a second pass, not left as
//! `EngineError::internal` everywhere. These drive a couple of the
//! classified failure paths through the real engine handlers
//! (`EngineHandlerHarness`) and pin the `kind` a control-API consumer
//! would branch on — not just that *an* error fired.

use resonance_audio::test_support::{EngineHandlerHarness, MAX_BUSSES};
use resonance_audio::types::{AudioEvent, ChainOwner, EngineErrorKind};

fn error_kind(events: &[AudioEvent]) -> Option<EngineErrorKind> {
    events.iter().find_map(|e| match e {
        AudioEvent::Error(err) => Some(err.kind),
        _ => None,
    })
}

/// Reordering a plugin instance the master chain doesn't have is a
/// `NotFound` (`engine/chain.rs::handle_move_plugin`), not the
/// historical bare string.
#[test]
fn unknown_plugin_on_master_reorder_is_not_found() {
    let mut harness = EngineHandlerHarness::new();
    harness.move_plugin(ChainOwner::Master, 9999, 0);
    let events = harness.drain_events();
    assert_eq!(
        error_kind(&events),
        Some(EngineErrorKind::NotFound),
        "events: {events:?}"
    );
}

/// Past `MAX_BUSSES`, `AddBus` refuses with `Busy` — the engine cannot
/// service the request right now (`engine/busses.rs::handle_add_bus`),
/// which is a different situation from an unexpected internal failure.
#[test]
fn bus_limit_reached_is_busy() {
    let mut harness = EngineHandlerHarness::new();
    // The app allocates every bus id now (ARCH-04 D-3), so each add needs
    // its own — a repeated id would be refused as a duplicate before ever
    // reaching the `MAX_BUSSES` check this test is after.
    for id in 1..=MAX_BUSSES as u64 {
        harness.add_bus(id, None);
    }
    let events = harness.add_bus(MAX_BUSSES as u64 + 1, None);
    assert_eq!(
        error_kind(&events),
        Some(EngineErrorKind::Busy),
        "events: {events:?}"
    );
}
