//! The engine half of MIDI Learn (doc #167 §2 E3): the active binding set
//! and its echoes, the learn arm, and what a control-surface message turns
//! into. Driven through the real dispatch and the real drain handler.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioCommand, AudioEvent};
use resonance_common::{
    BindingId, CcMode, ControlSource, ControllerMap, MidiBinding, MidiTarget, RelativeEnc,
    TransportAction,
};

fn cc(id: u64, channel: u8, cc: u8, target: MidiTarget) -> MidiBinding {
    MidiBinding::new(
        BindingId(id),
        ControlSource::Cc {
            channel,
            cc,
            mode: CcMode::Absolute,
        },
        target,
    )
}

fn midi_map_events(h: &mut EngineHandlerHarness) -> Vec<AudioEvent> {
    h.drain_events()
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                AudioEvent::MidiBindingChanged { .. }
                    | AudioEvent::MidiBindingCleared { .. }
                    | AudioEvent::MidiLearnCaptured { .. }
                    | AudioEvent::ControlSurfaceMoved { .. }
            )
        })
        .collect()
}

#[test]
fn set_binding_echoes_and_displaces_the_controls_previous_binding() {
    let mut h = EngineHandlerHarness::new();
    let first = cc(1, 0, 7, MidiTarget::TrackVolume(3));
    h.dispatch(AudioCommand::SetMidiBinding { binding: first });
    assert!(matches!(
        midi_map_events(&mut h).as_slice(),
        [AudioEvent::MidiBindingChanged { binding }] if *binding == first
    ));

    // Same physical control (channel 0, CC 7) — even read as an encoder —
    // now drives the pan: the volume binding goes, and says so.
    let mut second = cc(2, 0, 7, MidiTarget::TrackPan(3));
    second.source = ControlSource::Cc {
        channel: 0,
        cc: 7,
        mode: CcMode::Relative(RelativeEnc::TwosComplement),
    };
    h.dispatch(AudioCommand::SetMidiBinding { binding: second });
    let events = midi_map_events(&mut h);
    assert!(
        matches!(
            events.as_slice(),
            [
                AudioEvent::MidiBindingCleared { id: BindingId(1) },
                AudioEvent::MidiBindingChanged { binding },
            ] if *binding == second
        ),
        "{events:?}"
    );

    // The same control on another channel is another control.
    h.dispatch(AudioCommand::SetMidiBinding {
        binding: cc(3, 1, 7, MidiTarget::TrackVolume(4)),
    });
    assert_eq!(midi_map_events(&mut h).len(), 1);
}

#[test]
fn clear_and_clear_all_echo_only_what_existed() {
    let mut h = EngineHandlerHarness::new();
    for (id, n) in [(1, 7), (2, 10)] {
        h.dispatch(AudioCommand::SetMidiBinding {
            binding: cc(id, 0, n, MidiTarget::TrackVolume(id)),
        });
    }
    let _ = midi_map_events(&mut h);

    h.dispatch(AudioCommand::ClearMidiBinding { id: BindingId(9) });
    assert!(midi_map_events(&mut h).is_empty(), "no such binding: silent");

    h.dispatch(AudioCommand::ClearMidiBinding { id: BindingId(1) });
    assert!(matches!(
        midi_map_events(&mut h).as_slice(),
        [AudioEvent::MidiBindingCleared { id: BindingId(1) }]
    ));

    h.dispatch(AudioCommand::ClearAllMidiBindings);
    assert!(matches!(
        midi_map_events(&mut h).as_slice(),
        [AudioEvent::MidiBindingCleared { id: BindingId(2) }]
    ));
    // A cleared control no longer reports moves.
    h.control_surface_message(&[0xB0, 10, 64]);
    assert!(midi_map_events(&mut h).is_empty());
}

#[test]
fn a_controller_map_replaces_the_whole_set() {
    let mut h = EngineHandlerHarness::new();
    h.dispatch(AudioCommand::SetMidiBinding {
        binding: cc(1, 0, 7, MidiTarget::TrackVolume(1)),
    });
    let _ = midi_map_events(&mut h);

    let map = ControllerMap {
        name: "Project".into(),
        bindings: vec![
            cc(5, 0, 20, MidiTarget::TrackPan(1)),
            cc(6, 0, 21, MidiTarget::TrackPan(2)),
        ],
    };
    h.dispatch(AudioCommand::SetControllerMap { map });
    let events = midi_map_events(&mut h);
    assert!(
        matches!(
            events.as_slice(),
            [
                AudioEvent::MidiBindingCleared { id: BindingId(1) },
                AudioEvent::MidiBindingChanged { binding: a },
                AudioEvent::MidiBindingChanged { binding: b },
            ] if a.id == BindingId(5) && b.id == BindingId(6)
        ),
        "{events:?}"
    );
}

#[test]
fn learn_captures_the_first_cc_or_note_on_and_disarms() {
    let mut h = EngineHandlerHarness::new();
    let target = MidiTarget::TrackVolume(3);
    h.dispatch(AudioCommand::EnterMidiLearn { target });

    // A note release is never what the user meant to learn.
    h.control_surface_message(&[0x80, 60, 0]);
    assert!(midi_map_events(&mut h).is_empty());

    h.control_surface_message(&[0xB2, 74, 100]);
    assert!(matches!(
        midi_map_events(&mut h).as_slice(),
        [AudioEvent::MidiLearnCaptured {
            target: t,
            source: ControlSource::Cc { channel: 2, cc: 74, mode: CcMode::Absolute },
        }] if *t == target
    ));

    // Disarmed: the next message is just an unbound move.
    h.control_surface_message(&[0xB2, 74, 90]);
    assert!(midi_map_events(&mut h).is_empty());

    // A pad learns as a note.
    let transport = MidiTarget::Transport(TransportAction::Play);
    h.dispatch(AudioCommand::EnterMidiLearn { target: transport });
    h.control_surface_message(&[0x99, 36, 127]);
    assert!(matches!(
        midi_map_events(&mut h).as_slice(),
        [AudioEvent::MidiLearnCaptured {
            source: ControlSource::Note { channel: 9, note: 36 },
            ..
        }]
    ));
}

#[test]
fn cancelled_learn_captures_nothing() {
    let mut h = EngineHandlerHarness::new();
    h.dispatch(AudioCommand::EnterMidiLearn {
        target: MidiTarget::TrackPan(1),
    });
    h.dispatch(AudioCommand::CancelMidiLearn);
    h.control_surface_message(&[0xB0, 7, 64]);
    assert!(midi_map_events(&mut h).is_empty());
}

#[test]
fn a_bound_control_reports_its_raw_value_and_notes_fire_on_press_only() {
    let mut h = EngineHandlerHarness::new();
    let fader = cc(1, 0, 7, MidiTarget::TrackVolume(3));
    let pad = MidiBinding::new(
        BindingId(2),
        ControlSource::Note { channel: 0, note: 36 },
        MidiTarget::TrackMute(3),
    );
    h.dispatch(AudioCommand::SetMidiBinding { binding: fader });
    h.dispatch(AudioCommand::SetMidiBinding { binding: pad });
    let _ = midi_map_events(&mut h);

    h.control_surface_message(&[0xB0, 7, 101]);
    // Unbound CC and the bound CC on another channel: nothing.
    h.control_surface_message(&[0xB0, 8, 101]);
    h.control_surface_message(&[0xB1, 7, 101]);
    h.control_surface_message(&[0x90, 36, 90]);
    // Release (note-off, and note-on velocity 0): nothing.
    h.control_surface_message(&[0x80, 36, 0]);
    h.control_surface_message(&[0x90, 36, 0]);

    let events = midi_map_events(&mut h);
    assert!(
        matches!(
            events.as_slice(),
            [
                AudioEvent::ControlSurfaceMoved { binding: a, value: 101 },
                AudioEvent::ControlSurfaceMoved { binding: b, value: 90 },
            ] if *a == fader && *b == pad
        ),
        "{events:?}"
    );
}
