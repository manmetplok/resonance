//! Contract tests for the MIDI Learn / hardware-controller-mapping
//! command & event plumbing (todo #429, doc #167 §2 E2).
//!
//! This slice only wires the command/event variants and stub handlers; the
//! real binding application lands in E3. So these tests pin the *shape* of
//! the new public boundary — that every command and event variant exists,
//! carries the documented payload built from the shared `resonance-common`
//! `midi_map` types, and survives the `Clone` + `Debug` the engine channel
//! relies on. A renamed field or dropped variant fails to compile here, and
//! the dispatch table's exhaustiveness (no wildcard arm) is enforced by the
//! crate building at all.

use resonance_audio::types::{AudioCommand, AudioEvent};
use resonance_common::{
    BindingId, CcMode, ControlSource, ControllerMap, MidiBinding, MidiTarget, TransportAction,
};

/// A representative absolute-CC binding driving a track's volume.
fn sample_binding() -> MidiBinding {
    MidiBinding::new(
        BindingId(7),
        ControlSource::Cc {
            channel: 0,
            cc: 21,
            mode: CcMode::Absolute,
        },
        MidiTarget::TrackVolume(3),
    )
}

#[test]
fn binding_commands_round_trip_through_clone() {
    let binding = sample_binding();

    // SetMidiBinding carries the full binding.
    match (AudioCommand::SetMidiBinding { binding }).clone() {
        AudioCommand::SetMidiBinding { binding: b } => {
            assert_eq!(b.id, BindingId(7));
            assert_eq!(b.target, MidiTarget::TrackVolume(3));
        }
        other => panic!("expected SetMidiBinding, got {other:?}"),
    }

    // ClearMidiBinding carries the id.
    match (AudioCommand::ClearMidiBinding { id: BindingId(7) }).clone() {
        AudioCommand::ClearMidiBinding { id } => assert_eq!(id, BindingId(7)),
        other => panic!("expected ClearMidiBinding, got {other:?}"),
    }

    // SetControllerMap carries a named preset of bindings.
    let map = ControllerMap {
        name: "Launch Control".into(),
        bindings: vec![sample_binding()],
    };
    match (AudioCommand::SetControllerMap { map }).clone() {
        AudioCommand::SetControllerMap { map } => {
            assert_eq!(map.name, "Launch Control");
            assert_eq!(map.bindings.len(), 1);
        }
        other => panic!("expected SetControllerMap, got {other:?}"),
    }

    // ClearAllMidiBindings is a unit variant.
    assert!(matches!(
        AudioCommand::ClearAllMidiBindings.clone(),
        AudioCommand::ClearAllMidiBindings
    ));
}

#[test]
fn surface_and_learn_commands_carry_their_payloads() {
    // SetControlSurfaceInput picks (Some) or clears (None) the port.
    match (AudioCommand::SetControlSurfaceInput {
        device: Some("APC mini".into()),
    })
    .clone()
    {
        AudioCommand::SetControlSurfaceInput { device } => {
            assert_eq!(device.as_deref(), Some("APC mini"))
        }
        other => panic!("expected SetControlSurfaceInput, got {other:?}"),
    }
    assert!(matches!(
        AudioCommand::SetControlSurfaceInput { device: None },
        AudioCommand::SetControlSurfaceInput { device: None }
    ));

    // EnterMidiLearn arms a target; CancelMidiLearn is a unit variant.
    match (AudioCommand::EnterMidiLearn {
        target: MidiTarget::Transport(TransportAction::Play),
    })
    .clone()
    {
        AudioCommand::EnterMidiLearn { target } => {
            assert_eq!(target, MidiTarget::Transport(TransportAction::Play))
        }
        other => panic!("expected EnterMidiLearn, got {other:?}"),
    }
    assert!(matches!(
        AudioCommand::CancelMidiLearn.clone(),
        AudioCommand::CancelMidiLearn
    ));
}

#[test]
fn midi_map_events_round_trip_through_clone() {
    let binding = sample_binding();
    let source = ControlSource::Note {
        channel: 0,
        note: 60,
    };

    match (AudioEvent::MidiLearnCaptured {
        target: MidiTarget::TrackMute(4),
        source,
    })
    .clone()
    {
        AudioEvent::MidiLearnCaptured { target, source } => {
            assert_eq!(target, MidiTarget::TrackMute(4));
            assert_eq!(source, ControlSource::Note { channel: 0, note: 60 });
        }
        other => panic!("expected MidiLearnCaptured, got {other:?}"),
    }

    match (AudioEvent::MidiBindingChanged { binding }).clone() {
        AudioEvent::MidiBindingChanged { binding } => assert_eq!(binding.id, BindingId(7)),
        other => panic!("expected MidiBindingChanged, got {other:?}"),
    }

    match (AudioEvent::MidiBindingCleared { id: BindingId(7) }).clone() {
        AudioEvent::MidiBindingCleared { id } => assert_eq!(id, BindingId(7)),
        other => panic!("expected MidiBindingCleared, got {other:?}"),
    }

    match (AudioEvent::ControlSurfaceParamChanged {
        target: MidiTarget::TrackPan(2),
        value_norm: 0.75,
    })
    .clone()
    {
        AudioEvent::ControlSurfaceParamChanged { target, value_norm } => {
            assert_eq!(target, MidiTarget::TrackPan(2));
            assert!((value_norm - 0.75).abs() < f32::EPSILON);
        }
        other => panic!("expected ControlSurfaceParamChanged, got {other:?}"),
    }

    match (AudioEvent::ControlSurfaceDevicesChanged {
        inputs: vec!["APC mini".into(), "nanoKONTROL2".into()],
    })
    .clone()
    {
        AudioEvent::ControlSurfaceDevicesChanged { inputs } => {
            assert_eq!(inputs, vec!["APC mini".to_string(), "nanoKONTROL2".to_string()]);
        }
        other => panic!("expected ControlSurfaceDevicesChanged, got {other:?}"),
    }
}
