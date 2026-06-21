use resonance_common::midi_map::*;

// --- Encoder decode: all three relative formats -------------------------------

#[test]
fn decode_twos_complement() {
    // 0..=63 are positive deltas; 64..=127 wrap to negative.
    assert_eq!(decode_relative(RelativeEnc::TwosComplement, 0), 0);
    assert_eq!(decode_relative(RelativeEnc::TwosComplement, 1), 1);
    assert_eq!(decode_relative(RelativeEnc::TwosComplement, 63), 63);
    assert_eq!(decode_relative(RelativeEnc::TwosComplement, 127), -1);
    assert_eq!(decode_relative(RelativeEnc::TwosComplement, 65), -63);
    assert_eq!(decode_relative(RelativeEnc::TwosComplement, 64), -64);
}

#[test]
fn decode_signed_bit() {
    // Bit 6 is the sign (set => increment); bits 0-5 the magnitude.
    assert_eq!(decode_relative(RelativeEnc::SignedBit, 0x41), 1);
    assert_eq!(decode_relative(RelativeEnc::SignedBit, 0x7F), 63);
    assert_eq!(decode_relative(RelativeEnc::SignedBit, 0x01), -1);
    assert_eq!(decode_relative(RelativeEnc::SignedBit, 0x3F), -63);
    // Magnitude zero either way is no movement.
    assert_eq!(decode_relative(RelativeEnc::SignedBit, 0x40), 0);
    assert_eq!(decode_relative(RelativeEnc::SignedBit, 0x00), 0);
}

#[test]
fn decode_binary_offset() {
    // Centered on 64.
    assert_eq!(decode_relative(RelativeEnc::BinaryOffset, 64), 0);
    assert_eq!(decode_relative(RelativeEnc::BinaryOffset, 65), 1);
    assert_eq!(decode_relative(RelativeEnc::BinaryOffset, 63), -1);
    assert_eq!(decode_relative(RelativeEnc::BinaryOffset, 127), 63);
    assert_eq!(decode_relative(RelativeEnc::BinaryOffset, 0), -64);
}

// --- Absolute CC mapping ------------------------------------------------------

#[test]
fn cc_to_norm_full_range() {
    assert_eq!(cc_to_norm(0, 0.0, 1.0, false), 0.0);
    assert_eq!(cc_to_norm(127, 0.0, 1.0, false), 1.0);
    assert!((cc_to_norm(64, 0.0, 1.0, false) - 64.0 / 127.0).abs() < 1e-6);
}

#[test]
fn cc_to_norm_invert_flips_ends() {
    assert_eq!(cc_to_norm(0, 0.0, 1.0, true), 1.0);
    assert_eq!(cc_to_norm(127, 0.0, 1.0, true), 0.0);
}

#[test]
fn cc_to_norm_sub_range() {
    // A control limited to the top half of the range.
    assert_eq!(cc_to_norm(0, 0.5, 1.0, false), 0.5);
    assert_eq!(cc_to_norm(127, 0.5, 1.0, false), 1.0);
    assert!((cc_to_norm(127, 0.5, 1.0, true) - 0.5).abs() < 1e-6);
}

// --- Relative apply -----------------------------------------------------------

#[test]
fn apply_delta_moves_and_clamps() {
    // A +127 sweep covers the whole 0..1 span.
    assert!((apply_delta(0.0, 127, 0.0, 1.0, false) - 1.0).abs() < 1e-6);
    // Halfway up from the bottom.
    assert!((apply_delta(0.0, 64, 0.0, 1.0, false) - 64.0 / 127.0).abs() < 1e-6);
    // Clamps at the top of the range.
    assert_eq!(apply_delta(1.0, 10, 0.0, 1.0, false), 1.0);
    // Negative delta decreases, clamped at the bottom.
    assert_eq!(apply_delta(0.0, -10, 0.0, 1.0, false), 0.0);
}

#[test]
fn apply_delta_invert_flips_direction() {
    let up = apply_delta(0.5, 10, 0.0, 1.0, false);
    let down = apply_delta(0.5, 10, 0.0, 1.0, true);
    assert!(up > 0.5);
    assert!(down < 0.5);
    assert!((up - 0.5).abs() - (0.5 - down).abs() < 1e-6);
}

#[test]
fn apply_delta_respects_sub_range() {
    // Cannot exceed the binding's max even on a big sweep.
    assert_eq!(apply_delta(0.5, 127, 0.0, 0.75, false), 0.75);
    assert_eq!(apply_delta(0.5, -127, 0.25, 1.0, false), 0.25);
}

// --- Soft takeover modes ------------------------------------------------------

#[test]
fn takeover_jump_always_adopts() {
    assert_eq!(takeover_value(Takeover::Jump, 0.9, 0.1), Some(0.9));
    assert_eq!(takeover_value(Takeover::Jump, 0.0, 1.0), Some(0.0));
}

#[test]
fn takeover_pickup_swallows_until_close() {
    // Far apart => swallow (None).
    assert_eq!(takeover_value(Takeover::Pickup, 0.9, 0.1), None);
    // Within tolerance => engage.
    assert_eq!(takeover_value(Takeover::Pickup, 0.5, 0.5), Some(0.5));
    let near = 0.5 + PICKUP_TOLERANCE / 2.0;
    assert_eq!(takeover_value(Takeover::Pickup, near, 0.5), Some(near));
    // Just outside tolerance => still swallowed.
    let far = 0.5 + PICKUP_TOLERANCE * 2.0;
    assert_eq!(takeover_value(Takeover::Pickup, far, 0.5), None);
}

#[test]
fn takeover_scale_converges_to_target() {
    // One step moves SCALE_RATE of the way from current toward incoming.
    let v = takeover_value(Takeover::Scale, 1.0, 0.0).unwrap();
    assert!((v - SCALE_RATE).abs() < 1e-6);

    // Repeated identical incoming values converge on the incoming value.
    let mut current = 0.0;
    for _ in 0..40 {
        current = takeover_value(Takeover::Scale, 1.0, current).unwrap();
    }
    assert!((current - 1.0).abs() < 1e-3);

    // Already at the value => returns it unchanged.
    assert_eq!(takeover_value(Takeover::Scale, 0.5, 0.5), Some(0.5));
}

// --- Defaults -----------------------------------------------------------------

#[test]
fn binding_new_uses_sensible_defaults() {
    let b = MidiBinding::new(
        BindingId(7),
        ControlSource::Cc {
            channel: 0,
            cc: 21,
            mode: CcMode::Absolute,
        },
        MidiTarget::TrackVolume(3),
    );
    assert_eq!(b.min, 0.0);
    assert_eq!(b.max, 1.0);
    assert!(!b.invert);
    assert_eq!(b.takeover, Takeover::Jump);
    assert_eq!(Takeover::default(), Takeover::Jump);
}

// --- Serde round-trips --------------------------------------------------------

#[test]
fn binding_serde_round_trip() {
    let bindings = vec![
        MidiBinding {
            id: BindingId(1),
            source: ControlSource::Cc {
                channel: 2,
                cc: 74,
                mode: CcMode::Relative(RelativeEnc::TwosComplement),
            },
            target: MidiTarget::PluginParam {
                instance: 9,
                param_id: 42,
            },
            min: 0.1,
            max: 0.9,
            invert: true,
            takeover: Takeover::Pickup,
        },
        MidiBinding {
            id: BindingId(2),
            source: ControlSource::Note {
                channel: 0,
                note: 60,
            },
            target: MidiTarget::Transport(TransportAction::LoopToggle),
            min: 0.0,
            max: 1.0,
            invert: false,
            takeover: Takeover::Jump,
        },
        MidiBinding {
            id: BindingId(3),
            source: ControlSource::Cc {
                channel: 1,
                cc: 7,
                mode: CcMode::Absolute,
            },
            target: MidiTarget::SendLevel {
                track: 5,
                send: SendId(2),
            },
            min: 0.0,
            max: 1.0,
            invert: false,
            takeover: Takeover::Scale,
        },
    ];
    for b in &bindings {
        let json = serde_json::to_string(b).unwrap();
        let back: MidiBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, b);
    }
}

#[test]
fn id_newtypes_serialize_transparently() {
    // serde(transparent) => a plain number, not a wrapper object.
    assert_eq!(serde_json::to_string(&BindingId(42)).unwrap(), "42");
    assert_eq!(serde_json::to_string(&SendId(7)).unwrap(), "7");
    assert_eq!(
        serde_json::from_str::<BindingId>("42").unwrap(),
        BindingId(42)
    );
}

#[test]
fn controller_map_serde_round_trip() {
    let map = ControllerMap {
        name: "Launch Control".to_string(),
        bindings: vec![MidiBinding::new(
            BindingId(1),
            ControlSource::Cc {
                channel: 0,
                cc: 21,
                mode: CcMode::Absolute,
            },
            MidiTarget::TrackPan(0),
        )],
    };
    let json = serde_json::to_string_pretty(&map).unwrap();
    let back: ControllerMap = serde_json::from_str(&json).unwrap();
    assert_eq!(back, map);
}

// --- Preset file I/O ----------------------------------------------------------

fn sample_map(name: &str) -> ControllerMap {
    ControllerMap {
        name: name.to_string(),
        bindings: vec![MidiBinding::new(
            BindingId(1),
            ControlSource::Cc {
                channel: 0,
                cc: 7,
                mode: CcMode::Absolute,
            },
            MidiTarget::TrackVolume(0),
        )],
    }
}

#[test]
fn preset_file_save_load_replace_delete() {
    let dir = std::env::temp_dir().join("resonance_test_controller_maps");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("controller_maps.json");

    // Missing file loads as empty.
    assert!(load_controller_maps_from(&path).is_empty());

    // Save two maps.
    save_controller_map_to(&sample_map("A"), &path).unwrap();
    save_controller_map_to(&sample_map("B"), &path).unwrap();
    let loaded = load_controller_maps_from(&path);
    assert_eq!(loaded.len(), 2);

    // Re-saving the same name replaces rather than duplicates.
    let mut changed = sample_map("A");
    changed.bindings.clear();
    save_controller_map_to(&changed, &path).unwrap();
    let loaded = load_controller_maps_from(&path);
    assert_eq!(loaded.len(), 2);
    let a = loaded.iter().find(|m| m.name == "A").unwrap();
    assert!(a.bindings.is_empty());

    // Delete one.
    delete_controller_map_from("A", &path).unwrap();
    let loaded = load_controller_maps_from(&path);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "B");

    // Deleting a missing map is a no-op success.
    delete_controller_map_from("missing", &path).unwrap();
    assert_eq!(load_controller_maps_from(&path).len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_preset_file_loads_empty() {
    let dir = std::env::temp_dir().join("resonance_test_controller_maps_corrupt");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("controller_maps.json");
    std::fs::write(&path, b"{ not valid json").unwrap();
    assert!(load_controller_maps_from(&path).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
