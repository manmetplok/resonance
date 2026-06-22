//! Aux-send / return-bus update handlers (ba todo #477).
//!
//! Each [`MixerMessage`] is dispatched against a `Resonance` whose engine
//! has been swapped for a command-capturing stub, so the tests assert the
//! exact `AudioCommand`(s) the handler emits. The handlers never touch the
//! send graph themselves — the engine echoes drive it — so state-mirroring
//! is covered separately by `tests/aux_send_mirror.rs`.

use resonance_app::message::{Message, MixerMessage};
use resonance_app::undo::{classify, CoalesceKey, UndoAction};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, AuxSend, SendSource};

/// Build an app with a capturing engine; return the app and the receiver
/// the handlers' commands queue onto.
fn capturing_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn send(id: u64, source: SendSource, dest: u64, level_db: f32, pre_fader: bool, enabled: bool) -> AuxSend {
    AuxSend {
        id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled,
    }
}

#[test]
fn add_send_emits_set_aux_send_with_no_id_hint_and_defaults() {
    let (mut app, rx) = capturing_app();
    app.test_dispatch(Message::Mixer(MixerMessage::AddSend {
        source: SendSource::Track(7),
        dest: 10,
    }));

    let cmds = drain(&rx);
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        AudioCommand::SetAuxSend {
            id_hint,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => {
            assert_eq!(*id_hint, None, "a fresh send lets the engine allocate the id");
            assert_eq!(*source, SendSource::Track(7));
            assert_eq!(*dest, 10);
            assert_eq!(*level_db, 0.0);
            assert!(!*pre_fader);
            assert!(*enabled);
        }
        other => panic!("expected SetAuxSend, got {other:?}"),
    }
}

#[test]
fn remove_send_emits_remove_aux_send() {
    let (mut app, rx) = capturing_app();
    app.test_dispatch(Message::Mixer(MixerMessage::RemoveSend(42)));

    let cmds = drain(&rx);
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        AudioCommand::RemoveAuxSend { send_id } => assert_eq!(*send_id, 42),
        other => panic!("expected RemoveAuxSend, got {other:?}"),
    }
}

#[test]
fn set_send_level_upserts_under_existing_id_preserving_other_fields() {
    let (mut app, rx) = capturing_app();
    // A pre-fader, enabled send at -6 dB into bus 10.
    app.test_seed_aux_send(send(3, SendSource::Bus(20), 10, -6.0, true, true));

    app.test_dispatch(Message::Mixer(MixerMessage::SetSendLevel(3, -2.5)));

    let cmds = drain(&rx);
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        AudioCommand::SetAuxSend {
            id_hint,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => {
            assert_eq!(*id_hint, Some(3), "an edit upserts under the send's id");
            assert_eq!(*source, SendSource::Bus(20));
            assert_eq!(*dest, 10);
            assert_eq!(*level_db, -2.5, "only the level changed");
            assert!(*pre_fader, "pre-fader preserved");
            assert!(*enabled, "enabled preserved");
        }
        other => panic!("expected SetAuxSend, got {other:?}"),
    }
}

#[test]
fn toggle_pre_fader_flips_only_that_field() {
    let (mut app, rx) = capturing_app();
    app.test_seed_aux_send(send(5, SendSource::Track(1), 11, -3.0, false, true));

    app.test_dispatch(Message::Mixer(MixerMessage::ToggleSendPreFader(5)));

    let cmds = drain(&rx);
    match &cmds[0] {
        AudioCommand::SetAuxSend {
            id_hint,
            level_db,
            pre_fader,
            enabled,
            ..
        } => {
            assert_eq!(*id_hint, Some(5));
            assert_eq!(*level_db, -3.0);
            assert!(*pre_fader, "post -> pre");
            assert!(*enabled);
        }
        other => panic!("expected SetAuxSend, got {other:?}"),
    }
}

#[test]
fn toggle_enabled_flips_only_that_field() {
    let (mut app, rx) = capturing_app();
    app.test_seed_aux_send(send(6, SendSource::Track(1), 11, -3.0, false, true));

    app.test_dispatch(Message::Mixer(MixerMessage::ToggleSendEnabled(6)));

    match &drain(&rx)[0] {
        AudioCommand::SetAuxSend {
            id_hint, enabled, ..
        } => {
            assert_eq!(*id_hint, Some(6));
            assert!(!*enabled, "enabled -> disabled");
        }
        other => panic!("expected SetAuxSend, got {other:?}"),
    }
}

#[test]
fn set_send_dest_reroutes_to_new_bus() {
    let (mut app, rx) = capturing_app();
    app.test_seed_aux_send(send(8, SendSource::Track(1), 10, -3.0, false, true));

    app.test_dispatch(Message::Mixer(MixerMessage::SetSendDest(8, 99)));

    match &drain(&rx)[0] {
        AudioCommand::SetAuxSend { id_hint, dest, .. } => {
            assert_eq!(*id_hint, Some(8));
            assert_eq!(*dest, 99);
        }
        other => panic!("expected SetAuxSend, got {other:?}"),
    }
}

#[test]
fn editing_an_unknown_send_emits_nothing() {
    let (mut app, rx) = capturing_app();
    // No send with id 1 in the mirror.
    app.test_dispatch(Message::Mixer(MixerMessage::SetSendLevel(1, -4.0)));
    app.test_dispatch(Message::Mixer(MixerMessage::ToggleSendEnabled(1)));
    app.test_dispatch(Message::Mixer(MixerMessage::SetSendDest(1, 5)));

    assert!(
        drain(&rx).is_empty(),
        "an edit with nothing to base the upsert on is a no-op"
    );
}

#[test]
fn set_bus_return_role_emits_set_bus_role() {
    let (mut app, rx) = capturing_app();
    app.test_dispatch(Message::Mixer(MixerMessage::SetBusReturnRole(12, true)));

    match &drain(&rx)[0] {
        AudioCommand::SetBusRole { bus_id, is_return } => {
            assert_eq!(*bus_id, 12);
            assert!(*is_return);
        }
        other => panic!("expected SetBusRole, got {other:?}"),
    }
}

#[test]
fn create_return_from_send_adds_bus_flags_return_then_routes_send() {
    let (mut app, rx) = capturing_app();
    app.test_dispatch(Message::Mixer(MixerMessage::CreateReturnFromSend {
        source: SendSource::Track(4),
    }));

    let cmds = drain(&rx);
    assert_eq!(cmds.len(), 3, "add bus, flag return, route send — in order");

    let bus_id = match &cmds[0] {
        AudioCommand::AddBus { id_hint, name } => {
            let id = id_hint.expect("the app picks the new bus id up front");
            assert!(
                name.as_deref().unwrap_or_default().contains("Return"),
                "return bus gets a descriptive name, got {name:?}"
            );
            id
        }
        other => panic!("expected AddBus first, got {other:?}"),
    };
    match &cmds[1] {
        AudioCommand::SetBusRole { bus_id: b, is_return } => {
            assert_eq!(*b, bus_id, "the role is set on the bus just added");
            assert!(*is_return);
        }
        other => panic!("expected SetBusRole second, got {other:?}"),
    }
    match &cmds[2] {
        AudioCommand::SetAuxSend {
            id_hint,
            source,
            dest,
            enabled,
            ..
        } => {
            assert_eq!(*id_hint, None, "the send itself is fresh");
            assert_eq!(*source, SendSource::Track(4));
            assert_eq!(*dest, bus_id, "the send routes into the new return bus");
            assert!(*enabled);
        }
        other => panic!("expected SetAuxSend third, got {other:?}"),
    }
}

#[test]
fn successive_return_creations_pick_distinct_bus_ids() {
    let (mut app, rx) = capturing_app();
    app.test_dispatch(Message::Mixer(MixerMessage::CreateReturnFromSend {
        source: SendSource::Track(1),
    }));
    app.test_dispatch(Message::Mixer(MixerMessage::CreateReturnFromSend {
        source: SendSource::Track(2),
    }));

    let cmds = drain(&rx);
    let bus_ids: Vec<u64> = cmds
        .iter()
        .filter_map(|c| match c {
            AudioCommand::AddBus { id_hint, .. } => *id_hint,
            _ => None,
        })
        .collect();
    assert_eq!(bus_ids.len(), 2);
    assert_ne!(bus_ids[0], bus_ids[1], "each return bus gets its own id");
}

// ---------------------------------------------------------------------
// Undo classification
// ---------------------------------------------------------------------

#[test]
fn send_level_drag_coalesces_per_send() {
    let action = classify(&Message::Mixer(MixerMessage::SetSendLevel(9, -3.0)));
    assert!(
        matches!(action, UndoAction::RecordCoalesced(CoalesceKey::SendLevel(9))),
        "a level drag coalesces into one undo entry keyed by the send id, got {action:?}"
    );
}

#[test]
fn discrete_send_actions_record_atomic_undo_entries() {
    let discrete = [
        Message::Mixer(MixerMessage::AddSend {
            source: SendSource::Track(1),
            dest: 10,
        }),
        Message::Mixer(MixerMessage::RemoveSend(1)),
        Message::Mixer(MixerMessage::SetSendDest(1, 2)),
        Message::Mixer(MixerMessage::ToggleSendPreFader(1)),
        Message::Mixer(MixerMessage::ToggleSendEnabled(1)),
        Message::Mixer(MixerMessage::SetBusReturnRole(10, true)),
        Message::Mixer(MixerMessage::CreateReturnFromSend {
            source: SendSource::Track(1),
        }),
    ];
    for msg in discrete {
        let action = classify(&msg);
        assert!(
            matches!(action, UndoAction::Record),
            "{msg:?} should record an atomic undo entry, got {action:?}"
        );
    }
}
