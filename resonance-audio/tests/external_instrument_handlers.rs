//! Tests for the external-instrument command boundary (todo #450, doc #169).
//!
//! Drives the engine-internal pure helpers `set_external_instrument_in_place`,
//! `clear_external_instrument_in_place`, `set_external_instrument_latency_in_place`,
//! `set_external_instrument_patch_in_place` and
//! `check_external_instrument_devices_in_place` directly via the
//! `#[doc(hidden)]` re-exports. That keeps the test headless — no cpal stream,
//! no engine thread, no MIDI port — while exercising the exact store/replace /
//! clear / latency-and-patch update / device-offline reporting the
//! `AudioCommand::SetExternalInstrument*` / `ClearExternalInstrument` /
//! `CheckExternalInstrumentDevices` dispatch path runs.

use std::collections::HashSet;

use crossbeam_channel::unbounded;

use resonance_audio::types::AudioEvent;
use resonance_audio::{
    check_external_instrument_devices_in_place, clear_external_instrument_in_place,
    set_external_instrument_in_place, set_external_instrument_latency_in_place,
    set_external_instrument_patch_in_place, ExternalInstruments, MidiOutputRegistry,
};
use resonance_common::ExternalInstrument;

const TRACK: u64 = 7;

fn names(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn set_stores_config_and_emits_changed() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    let mut config = ExternalInstrument::new(TRACK);
    config.program = Some(4);
    set_external_instrument_in_place(&mut instruments, &tx, config);

    match rx.try_recv() {
        Ok(AudioEvent::ExternalInstrumentChanged { config: echoed }) => {
            assert_eq!(echoed, config, "echoed config mirrors stored config");
        }
        other => panic!("expected ExternalInstrumentChanged, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly one event emitted");
    assert_eq!(
        instruments.get(&TRACK).copied(),
        Some(config),
        "config stored under its track id"
    );
}

#[test]
fn set_replaces_existing_config() {
    let mut instruments = ExternalInstruments::new();
    let (tx, _rx) = unbounded::<AudioEvent>();

    set_external_instrument_in_place(&mut instruments, &tx, ExternalInstrument::new(TRACK));
    let mut replacement = ExternalInstrument::new(TRACK);
    replacement.bank = Some(128);
    set_external_instrument_in_place(&mut instruments, &tx, replacement);

    assert_eq!(instruments.len(), 1, "still one entry for the track");
    assert_eq!(instruments[&TRACK], replacement, "config replaced wholesale");
}

#[test]
fn clear_emits_only_when_present() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    // Absent track: silent no-op.
    clear_external_instrument_in_place(&mut instruments, &tx, TRACK);
    assert!(rx.try_recv().is_err(), "clearing an absent track emits nothing");

    set_external_instrument_in_place(&mut instruments, &tx, ExternalInstrument::new(TRACK));
    let _ = rx.try_recv(); // drain the Changed echo

    clear_external_instrument_in_place(&mut instruments, &tx, TRACK);
    match rx.try_recv() {
        Ok(AudioEvent::ExternalInstrumentCleared { track_id }) => assert_eq!(track_id, TRACK),
        other => panic!("expected ExternalInstrumentCleared, got {other:?}"),
    }
    assert!(!instruments.contains_key(&TRACK), "config removed");
}

#[test]
fn set_latency_updates_and_echoes() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    set_external_instrument_in_place(&mut instruments, &tx, ExternalInstrument::new(TRACK));
    let _ = rx.try_recv();

    set_external_instrument_latency_in_place(&mut instruments, &tx, TRACK, -256);
    match rx.try_recv() {
        Ok(AudioEvent::ExternalInstrumentChanged { config }) => {
            assert_eq!(config.latency_offset_samples, -256);
        }
        other => panic!("expected ExternalInstrumentChanged, got {other:?}"),
    }
    assert_eq!(instruments[&TRACK].latency_offset_samples, -256);
}

#[test]
fn set_latency_no_op_when_not_external() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    set_external_instrument_latency_in_place(&mut instruments, &tx, TRACK, 512);
    assert!(rx.try_recv().is_err(), "no event for a non-external track");
    assert!(instruments.is_empty(), "nothing stored");
}

#[test]
fn patch_updates_config_and_reports_offline_route_preserved() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();
    // Empty registry: no device assigned -> the patch send finds no live
    // connection and reports the MIDI output as offline.
    let mut outputs = MidiOutputRegistry::new();

    set_external_instrument_in_place(&mut instruments, &tx, ExternalInstrument::new(TRACK));
    let _ = rx.try_recv();

    set_external_instrument_patch_in_place(
        &mut instruments,
        &tx,
        &mut outputs,
        TRACK,
        0,
        Some("Synth Port".to_string()),
        Some(128),
        Some(42),
    );

    // First: the config echo with the new bank/program.
    match rx.try_recv() {
        Ok(AudioEvent::ExternalInstrumentChanged { config }) => {
            assert_eq!(config.bank, Some(128));
            assert_eq!(config.program, Some(42));
        }
        other => panic!("expected ExternalInstrumentChanged, got {other:?}"),
    }
    // Then: the offline report, route preserved.
    match rx.try_recv() {
        Ok(AudioEvent::ExternalInstrumentMidiOutOffline { track_id, device }) => {
            assert_eq!(track_id, TRACK);
            assert_eq!(device.as_deref(), Some("Synth Port"));
        }
        other => panic!("expected ExternalInstrumentMidiOutOffline, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly two events emitted");

    // Route preserved: the config still carries the requested patch.
    let stored = instruments[&TRACK];
    assert_eq!(stored.bank, Some(128));
    assert_eq!(stored.program, Some(42));
}

#[test]
fn patch_no_op_when_not_external() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();
    let mut outputs = MidiOutputRegistry::new();

    set_external_instrument_patch_in_place(
        &mut instruments,
        &tx,
        &mut outputs,
        TRACK,
        0,
        None,
        Some(1),
        Some(1),
    );
    assert!(rx.try_recv().is_err(), "no event for a non-external track");
    assert!(instruments.is_empty());
}

#[test]
fn check_devices_reports_each_missing_endpoint() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();
    set_external_instrument_in_place(&mut instruments, &tx, ExternalInstrument::new(TRACK));
    let _ = rx.try_recv();

    // MIDI out present, return input gone.
    check_external_instrument_devices_in_place(
        &instruments,
        &tx,
        TRACK,
        Some("Synth Port"),
        Some("USB Mic"),
        &names(&["Synth Port"]),
        &names(&["Built-in"]),
    );
    match rx.try_recv() {
        Ok(AudioEvent::ExternalInstrumentReturnInputOffline { track_id, device }) => {
            assert_eq!(track_id, TRACK);
            assert_eq!(device.as_deref(), Some("USB Mic"));
        }
        other => panic!("expected ExternalInstrumentReturnInputOffline, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "only the missing endpoint reported");

    // Both gone -> both reported.
    check_external_instrument_devices_in_place(
        &instruments,
        &tx,
        TRACK,
        Some("Synth Port"),
        Some("USB Mic"),
        &names(&[]),
        &names(&[]),
    );
    let mut got: Vec<AudioEvent> = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        got.push(ev);
    }
    assert_eq!(got.len(), 2, "both endpoints reported offline");
    assert!(got
        .iter()
        .any(|e| matches!(e, AudioEvent::ExternalInstrumentMidiOutOffline { .. })));
    assert!(got
        .iter()
        .any(|e| matches!(e, AudioEvent::ExternalInstrumentReturnInputOffline { .. })));
}

#[test]
fn check_devices_silent_when_present_or_not_external() {
    let mut instruments = ExternalInstruments::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    // Not an external instrument -> no-op even with missing devices.
    check_external_instrument_devices_in_place(
        &instruments,
        &tx,
        TRACK,
        Some("Synth Port"),
        Some("USB Mic"),
        &names(&[]),
        &names(&[]),
    );
    assert!(rx.try_recv().is_err(), "non-external track is a no-op");

    // External instrument, both endpoints present -> silent.
    set_external_instrument_in_place(&mut instruments, &tx, ExternalInstrument::new(TRACK));
    let _ = rx.try_recv();
    check_external_instrument_devices_in_place(
        &instruments,
        &tx,
        TRACK,
        Some("Synth Port"),
        Some("USB Mic"),
        &names(&["Synth Port"]),
        &names(&["USB Mic"]),
    );
    assert!(rx.try_recv().is_err(), "present endpoints stay silent");
}

#[test]
fn patch_messages_build_bank_select_then_program_change() {
    let mut config = ExternalInstrument::new(TRACK);
    // Combined 14-bit bank 130 = MSB 1, LSB 2.
    config.bank = Some((1 << 7) | 2);
    config.program = Some(42);

    let msgs = config.patch_messages(3);
    assert_eq!(
        msgs,
        vec![
            vec![0xB0 | 3, 0, 1],  // Bank Select MSB
            vec![0xB0 | 3, 32, 2], // Bank Select LSB
            vec![0xC0 | 3, 42],    // Program Change
        ]
    );
}

#[test]
fn patch_messages_empty_when_nothing_selected() {
    let config = ExternalInstrument::new(TRACK);
    assert!(config.patch_messages(0).is_empty());
}
