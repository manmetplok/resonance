//! Tests for the `AudioCommand::SetTrackDeviceParams` command boundary
//! (epic #40, architecture doc #201 §4, todo #722).
//!
//! Drives the engine-internal pure helper `set_track_device_params_in_place`
//! directly via the `#[doc(hidden)]` re-export. That keeps the test
//! headless — no cpal stream, no engine thread, no audio device — while
//! exercising the exact engine-side map update + `TrackDeviceParamsApplied`
//! emission (and the missing-track no-op branch) the command dispatch runs.

use std::sync::Arc;

use crossbeam_channel::unbounded;
use indexmap::IndexMap;
use parking_lot::RwLock;

use resonance_audio::set_track_device_params_in_place;
use resonance_audio::types::{AudioEvent, Track, TrackId};
use resonance_common::device_definition::MidiBinding;
use resonance_common::{DeviceParam, ParamCurve};

/// One CC-bound device param.
fn cc_param(id: &str, cc: u8) -> DeviceParam {
    DeviceParam {
        id: id.to_string(),
        name: id.to_string(),
        group: None,
        binding: MidiBinding::Cc { cc },
        min: 0,
        max: 127,
        default: None,
        curve: ParamCurve::Linear,
    }
}

/// A tracks map holding a single track with the given id.
fn tracks_with(track_id: TrackId) -> Arc<RwLock<IndexMap<TrackId, Track>>> {
    let mut map = IndexMap::new();
    map.insert(track_id, Track::new(track_id, "Synth".to_string()));
    Arc::new(RwLock::new(map))
}

#[test]
fn set_track_device_params_updates_map_and_emits_event() {
    let tracks = tracks_with(7);
    let (tx, rx) = unbounded();

    set_track_device_params_in_place(
        &tracks,
        &tx,
        7,
        vec![cc_param("cutoff", 74), cc_param("reso", 71)],
    );

    // The engine-side map is updated, keyed by param id.
    {
        let guard = tracks.read();
        let track = guard.get(&7).unwrap();
        assert_eq!(track.device_params().len(), 2);
        assert_eq!(
            track.device_param("cutoff").unwrap().binding,
            MidiBinding::Cc { cc: 74 }
        );
    }

    // A confirming event is emitted with the resolved param ids in order.
    match rx.try_recv() {
        Ok(AudioEvent::TrackDeviceParamsApplied { track_id, param_ids }) => {
            assert_eq!(track_id, 7);
            assert_eq!(param_ids, vec!["cutoff".to_string(), "reso".to_string()]);
        }
        other => panic!("expected TrackDeviceParamsApplied, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly one event expected");
}

#[test]
fn empty_params_clears_map_and_still_confirms() {
    let tracks = tracks_with(3);
    let (tx, rx) = unbounded();

    // Seed a map, then clear it with an empty command.
    set_track_device_params_in_place(&tracks, &tx, 3, vec![cc_param("a", 1)]);
    let _ = rx.try_recv(); // drain the first confirmation

    set_track_device_params_in_place(&tracks, &tx, 3, vec![]);

    {
        let guard = tracks.read();
        assert!(guard.get(&3).unwrap().device_params().is_empty());
    }

    match rx.try_recv() {
        Ok(AudioEvent::TrackDeviceParamsApplied { track_id, param_ids }) => {
            assert_eq!(track_id, 3);
            assert!(param_ids.is_empty());
        }
        other => panic!("expected a clear confirmation, got {other:?}"),
    }
}

#[test]
fn unknown_track_is_silent_no_op() {
    let tracks = tracks_with(1);
    let (tx, rx) = unbounded();

    // Target a track that does not exist: no panic, no event, and the
    // existing track's map is untouched.
    set_track_device_params_in_place(&tracks, &tx, 999, vec![cc_param("x", 5)]);

    assert!(
        rx.try_recv().is_err(),
        "an unknown track id must not emit a ghost event"
    );
    assert!(tracks.read().get(&1).unwrap().device_params().is_empty());
}
