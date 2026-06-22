//! Engine→app mirroring of the MIDI Learn / hardware control-surface
//! mapping (todo #431, arch doc #167 §3 A1, epic #21). The engine owns the
//! active binding set and echoes every change back as events; these tests
//! prove that `MidiMapState` is rebuilt purely from those events — bindings
//! upsert/clear, the reverse source index stays consistent, learn mode is
//! cleared on capture, live values are stashed, and the device list refreshes.

use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;
use resonance_common::{
    BindingId, CcMode, ControlSource, MidiBinding, MidiTarget, Takeover,
};

/// A CC binding spanning the full range with the given id, source CC and
/// target.
fn cc_binding(id: u64, cc: u8, target: MidiTarget) -> MidiBinding {
    MidiBinding::new(
        BindingId(id),
        ControlSource::Cc {
            channel: 0,
            cc,
            mode: CcMode::Absolute,
        },
        target,
    )
}

#[test]
fn binding_changed_upserts_and_indexes_source() {
    let mut app = Resonance::new().0;
    let b = cc_binding(1, 7, MidiTarget::TrackVolume(42));

    app.test_apply_engine_event(AudioEvent::MidiBindingChanged { binding: b });

    let map = app.test_midi_map();
    assert_eq!(map.bindings.len(), 1);
    assert_eq!(map.bindings[&BindingId(1)].target, MidiTarget::TrackVolume(42));
    // Reverse index resolves the physical control back to the binding.
    assert_eq!(map.source_index.get(&b.source), Some(&BindingId(1)));
}

#[test]
fn binding_changed_replaces_in_place_and_remaps_source() {
    let mut app = Resonance::new().0;
    app.test_apply_engine_event(AudioEvent::MidiBindingChanged {
        binding: cc_binding(1, 7, MidiTarget::TrackVolume(42)),
    });

    // Same id, but the user re-points it at a different CC and target.
    let moved = cc_binding(1, 9, MidiTarget::TrackPan(42));
    let old_source = ControlSource::Cc {
        channel: 0,
        cc: 7,
        mode: CcMode::Absolute,
    };
    app.test_apply_engine_event(AudioEvent::MidiBindingChanged { binding: moved });

    let map = app.test_midi_map();
    assert_eq!(map.bindings.len(), 1, "replaced in place, not duplicated");
    assert_eq!(map.bindings[&BindingId(1)].target, MidiTarget::TrackPan(42));
    // The stale source no longer points anywhere; the new one does.
    assert_eq!(map.source_index.get(&old_source), None);
    assert_eq!(map.source_index.get(&moved.source), Some(&BindingId(1)));
}

#[test]
fn binding_cleared_removes_binding_and_source_entry() {
    let mut app = Resonance::new().0;
    let b = cc_binding(1, 7, MidiTarget::TrackVolume(42));
    app.test_apply_engine_event(AudioEvent::MidiBindingChanged { binding: b });

    app.test_apply_engine_event(AudioEvent::MidiBindingCleared { id: BindingId(1) });

    let map = app.test_midi_map();
    assert!(map.bindings.is_empty());
    assert_eq!(map.source_index.get(&b.source), None);
}

#[test]
fn clearing_an_unknown_binding_is_a_no_op() {
    let mut app = Resonance::new().0;
    app.test_apply_engine_event(AudioEvent::MidiBindingChanged {
        binding: cc_binding(1, 7, MidiTarget::TrackVolume(42)),
    });

    app.test_apply_engine_event(AudioEvent::MidiBindingCleared { id: BindingId(99) });

    // The real binding survives untouched.
    assert_eq!(app.test_midi_map().bindings.len(), 1);
}

#[test]
fn controller_map_replay_rebuilds_the_whole_set() {
    let mut app = Resonance::new().0;
    // A SetControllerMap / project-load replay arrives as a stream of
    // per-binding MidiBindingChanged events.
    for (id, cc, target) in [
        (1u64, 7u8, MidiTarget::TrackVolume(1)),
        (2, 8, MidiTarget::TrackVolume(2)),
        (3, 9, MidiTarget::TrackMute(1)),
    ] {
        app.test_apply_engine_event(AudioEvent::MidiBindingChanged {
            binding: cc_binding(id, cc, target),
        });
    }

    let map = app.test_midi_map();
    assert_eq!(map.bindings.len(), 3);
    assert_eq!(map.source_index.len(), 3);
}

#[test]
fn learn_captured_records_binding_and_clears_learn_mode() {
    let mut app = Resonance::new().0;
    let target = MidiTarget::TrackSolo(5);
    app.test_arm_midi_learn(target);
    assert_eq!(app.test_midi_map().learn_target, Some(target));

    let source = ControlSource::Note {
        channel: 0,
        note: 36,
    };
    app.test_apply_engine_event(AudioEvent::MidiLearnCaptured { target, source });

    let map = app.test_midi_map();
    // Learn mode exited.
    assert_eq!(map.learn_target, None);
    // A binding was recorded for the captured source/target, with the
    // default full-range / Jump-takeover shape.
    assert_eq!(map.bindings.len(), 1);
    let b = map.bindings.values().next().unwrap();
    assert_eq!(b.source, source);
    assert_eq!(b.target, target);
    assert_eq!(b.min, 0.0);
    assert_eq!(b.max, 1.0);
    assert!(!b.invert);
    assert_eq!(b.takeover, Takeover::Jump);
    assert_eq!(map.source_index.get(&source), Some(&b.id));
}

#[test]
fn learn_captured_allocates_ids_clear_of_existing_bindings() {
    let mut app = Resonance::new().0;
    // A project-loaded binding already occupies a high id.
    app.test_apply_engine_event(AudioEvent::MidiBindingChanged {
        binding: cc_binding(100, 7, MidiTarget::TrackVolume(1)),
    });

    let target = MidiTarget::TrackPan(2);
    app.test_apply_engine_event(AudioEvent::MidiLearnCaptured {
        target,
        source: ControlSource::Cc {
            channel: 1,
            cc: 20,
            mode: CcMode::Absolute,
        },
    });

    let map = app.test_midi_map();
    assert_eq!(map.bindings.len(), 2, "no id collision dropped a binding");
    // The freshly-learned id sits past the project-loaded one.
    let learned = map.bindings.values().find(|b| b.target == target).unwrap();
    assert!(learned.id.0 > 100, "learned id {} must clear 100", learned.id.0);
}

#[test]
fn control_surface_param_changed_stashes_live_value() {
    let mut app = Resonance::new().0;
    let target = MidiTarget::TrackVolume(3);

    app.test_apply_engine_event(AudioEvent::ControlSurfaceParamChanged {
        target,
        value_norm: 0.75,
    });

    assert_eq!(app.test_midi_map().live_values.get(&target), Some(&0.75));
}

#[test]
fn control_surface_devices_changed_refreshes_inputs() {
    let mut app = Resonance::new().0;

    app.test_apply_engine_event(AudioEvent::ControlSurfaceDevicesChanged {
        inputs: vec!["Launch Control".to_string(), "nanoKONTROL".to_string()],
    });
    assert_eq!(app.test_midi_map().available_inputs.len(), 2);

    // A later enumeration wholesale-replaces the list (hot-unplug).
    app.test_apply_engine_event(AudioEvent::ControlSurfaceDevicesChanged {
        inputs: vec!["Launch Control".to_string()],
    });
    assert_eq!(
        app.test_midi_map().available_inputs,
        vec!["Launch Control".to_string()]
    );
}
