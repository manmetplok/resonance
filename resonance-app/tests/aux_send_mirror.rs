//! Aux-send / return-bus engine-event mirroring (ba todo #478).
//!
//! Drives `AudioEvent`s through the real dispatch and asserts the app
//! reconstructs the send graph purely from events — no read-getters.

use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, SendSource};

/// Seed a return bus the way the live app would — via the engine event,
/// not a private setter — so the send tests route into a real bus.
fn add_bus(app: &mut Resonance, bus_id: u64) {
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id,
        name: format!("Bus {bus_id}"),
    });
}

fn changed(send_id: u64, source: SendSource, dest: u64, level_db: f32) -> AudioEvent {
    AudioEvent::AuxSendChanged {
        send_id,
        source,
        dest,
        level_db,
        pre_fader: false,
        enabled: true,
    }
}

#[test]
fn bus_role_changed_toggles_only_the_target_bus() {
    let (mut app, _task) = Resonance::new();
    add_bus(&mut app, 10);
    add_bus(&mut app, 11);

    app.test_apply_engine_event(AudioEvent::BusRoleChanged {
        bus_id: 11,
        is_return: true,
    });

    let busses = &app.test_registry().busses;
    assert!(!busses.iter().find(|b| b.id == 10).unwrap().is_return);
    assert!(busses.iter().find(|b| b.id == 11).unwrap().is_return);

    // Toggling back clears it.
    app.test_apply_engine_event(AudioEvent::BusRoleChanged {
        bus_id: 11,
        is_return: false,
    });
    assert!(!app
        .test_registry()
        .busses
        .iter()
        .find(|b| b.id == 11)
        .unwrap()
        .is_return);
}

#[test]
fn bus_role_changed_for_unknown_bus_is_a_noop() {
    let (mut app, _task) = Resonance::new();
    // No bus added — must not panic and must not invent a bus.
    app.test_apply_engine_event(AudioEvent::BusRoleChanged {
        bus_id: 99,
        is_return: true,
    });
    assert!(app.test_registry().busses.iter().all(|b| b.id != 99));
}

#[test]
fn send_changed_inserts_then_updates_in_place() {
    let (mut app, _task) = Resonance::new();
    add_bus(&mut app, 10);

    app.test_apply_engine_event(changed(1, SendSource::Track(5), 10, -6.0));
    assert_eq!(app.test_aux_sends().len(), 1);
    let s = &app.test_aux_sends()[0];
    assert_eq!(s.id, 1);
    assert_eq!(s.source, SendSource::Track(5));
    assert_eq!(s.dest, 10);
    assert_eq!(s.level_db, -6.0);

    // Same id again (an edit) mirrors the engine-clamped level and does
    // not create a duplicate.
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id: 1,
        source: SendSource::Track(5),
        dest: 10,
        level_db: 0.0,
        pre_fader: true,
        enabled: false,
    });
    assert_eq!(app.test_aux_sends().len(), 1);
    let s = &app.test_aux_sends()[0];
    assert_eq!(s.level_db, 0.0);
    assert!(s.pre_fader);
    assert!(!s.enabled);
}

#[test]
fn multiple_sends_keep_insertion_order() {
    let (mut app, _task) = Resonance::new();
    add_bus(&mut app, 10);
    add_bus(&mut app, 20);

    app.test_apply_engine_event(changed(1, SendSource::Track(5), 10, -3.0));
    app.test_apply_engine_event(changed(2, SendSource::Bus(20), 10, -9.0));

    let ids: Vec<u64> = app.test_aux_sends().iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![1, 2]);
}

#[test]
fn send_removed_drops_only_that_send() {
    let (mut app, _task) = Resonance::new();
    add_bus(&mut app, 10);
    app.test_apply_engine_event(changed(1, SendSource::Track(5), 10, -3.0));
    app.test_apply_engine_event(changed(2, SendSource::Track(6), 10, -3.0));

    app.test_apply_engine_event(AudioEvent::AuxSendRemoved { send_id: 1 });

    let ids: Vec<u64> = app.test_aux_sends().iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![2]);

    // Removing an unknown send is a no-op.
    app.test_apply_engine_event(AudioEvent::AuxSendRemoved { send_id: 999 });
    assert_eq!(app.test_aux_sends().len(), 1);
}

#[test]
fn rejected_send_is_forwarded_to_ui_then_cleared_by_success() {
    let (mut app, _task) = Resonance::new();
    add_bus(&mut app, 10);

    app.test_apply_engine_event(AudioEvent::AuxSendRejected {
        source: SendSource::Bus(10),
        dest: 10,
        reason: "a bus cannot send to itself".to_string(),
    });
    let rej = app.test_aux_last_rejection().expect("rejection recorded");
    assert_eq!(rej.source, SendSource::Bus(10));
    assert_eq!(rej.dest, 10);
    assert_eq!(rej.reason, "a bus cannot send to itself");
    // A rejected send is never registered.
    assert!(app.test_aux_sends().is_empty());

    // A subsequent successful send supersedes the error.
    app.test_apply_engine_event(changed(1, SendSource::Track(5), 10, -6.0));
    assert!(app.test_aux_last_rejection().is_none());
    assert_eq!(app.test_aux_sends().len(), 1);
}
