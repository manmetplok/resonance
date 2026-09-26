//! ARCH-04 D-2: the app allocates every aux-send id now
//! (`AuxSendState::allocate_send_id`), and `AudioCommand::SetAuxSend` —
//! previously a single upsert command taking an optional `id_hint` — is
//! split into `AddAuxSend` (a concrete, mandatory `id`; refused on a
//! collision) and `SetAuxSend` (edits an existing id in place; a quiet
//! no-op on an unknown one). Same shape as ARCH-04 D-1's plugin ids
//! (`tests/clap_host/plugin_id_duplicate_rejected.rs`), adapted for the
//! fact that a send id legitimately reappears on every edit of that send
//! — unlike a plugin instance id, which never should.
//!
//! Drives the real `handle_add_aux_send` / `handle_set_aux_send` handlers
//! via `EngineHandlerHarness`, so what's proven is the actual
//! `HandlerState::aux_sends` map and the actual `AudioEvent`s — not a
//! description of the rule.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioEvent, EngineErrorKind, SendSource};

fn error_kind(events: &[AudioEvent]) -> Option<EngineErrorKind> {
    events.iter().find_map(|e| match e {
        AudioEvent::Error(err) => Some(err.kind),
        _ => None,
    })
}

fn changed_level(events: &[AudioEvent]) -> Option<f32> {
    events.iter().find_map(|e| match e {
        AudioEvent::AuxSendChanged { level_db, .. } => Some(*level_db),
        _ => None,
    })
}

#[test]
fn a_duplicate_add_is_refused_and_does_not_disturb_the_live_send() {
    let mut harness = EngineHandlerHarness::new();
    harness.add_bus(100, Some("Reverb".to_string()));
    harness.add_bus(101, Some("Delay".to_string()));

    let first = harness.add_aux_send(1, SendSource::Bus(100), 101, 0.0, false, true);
    assert!(
        first
            .iter()
            .any(|e| matches!(e, AudioEvent::AuxSendChanged { send_id: 1, .. })),
        "events: {first:?}"
    );
    assert_eq!(harness.aux_send_ids(), vec![1]);

    // A second ADD asking for the SAME id is refused — not treated as an
    // edit of the send that id already names, and not accepted as a
    // second copy.
    let second = harness.add_aux_send(1, SendSource::Bus(101), 100, -6.0, true, true);
    assert_eq!(
        error_kind(&second),
        Some(EngineErrorKind::Internal),
        "a duplicate id on AddAuxSend is a caller invariant violation, not \
         a routing rejection — events: {second:?}"
    );
    assert!(
        !second
            .iter()
            .any(|e| matches!(e, AudioEvent::AuxSendChanged { .. })),
        "the refused add must not also emit AuxSendChanged: {second:?}"
    );

    // The original send is untouched: still the only entry, still routed
    // bus 100 -> 101 at unity — a silently-accepted "add" would have
    // rewritten it to 101 -> 100 at -6 dB instead.
    assert_eq!(harness.aux_send_ids(), vec![1], "no second send was added");
    let unchanged = harness.set_aux_send(1, SendSource::Bus(100), 101, 0.0, false, true);
    assert_eq!(
        changed_level(&unchanged),
        Some(0.0),
        "the live send's level must still be the first add's, not the \
         refused second add's -6 dB"
    );
}

#[test]
fn an_edit_of_an_unknown_id_is_a_quiet_no_op() {
    let mut harness = EngineHandlerHarness::new();
    harness.add_bus(100, Some("Reverb".to_string()));

    // No send with id 1 has ever been added — `SetAuxSend` is edit-only,
    // so this must not create one behind `AddAuxSend`'s back.
    let events = harness.set_aux_send(1, SendSource::Bus(100), 100, 0.0, false, true);
    assert!(
        events.is_empty(),
        "an edit naming no live send must be a quiet no-op: {events:?}"
    );
    assert!(harness.aux_send_ids().is_empty(), "no send was created");
}

#[test]
fn an_edit_reuses_its_own_id_without_being_refused_as_a_duplicate() {
    // The whole point of splitting `AddAuxSend` from `SetAuxSend`: an
    // edit legitimately resends the send's own id on every call (level
    // drag, re-route, toggle), and that must never trip the collision
    // guard that protects `AddAuxSend`.
    let mut harness = EngineHandlerHarness::new();
    harness.add_bus(100, Some("Reverb".to_string()));
    harness.add_bus(101, Some("Delay".to_string()));
    harness.add_aux_send(1, SendSource::Bus(100), 101, 0.0, false, true);

    let edited = harness.set_aux_send(1, SendSource::Bus(100), 101, -3.0, false, true);
    assert_eq!(
        error_kind(&edited),
        None,
        "an edit under the send's own id must not be refused: {edited:?}"
    );
    assert_eq!(changed_level(&edited), Some(-3.0));
    assert_eq!(harness.aux_send_ids(), vec![1], "the edit updated in place");
}
