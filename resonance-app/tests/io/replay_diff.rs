//! Integration tests for the diff-based undo/redo replay path.
//!
//! Covers two logical halves:
//!   - Structural-compatibility check (`structurally_compatible` + `id_set_eq`
//!     + `*_set_matches` helpers): verifies which project shapes allow the fast
//!     path and which fall back to the full clear-and-replay cycle.
//!   - Note-equality helper (`midi_notes_equal`): field-wise comparison of MIDI
//!     notes, exercised across equal/unequal/different-length slices.
//!
//! If the test surface grows significantly, splitting into two submodules
//! (one for each half above) would aid navigation, but the current ~230 lines
//! fit comfortably in one file.

use resonance_app::compose::DrumGroup;
use resonance_app::project::{ProjectBus, ProjectClip, ProjectFile, ProjectPlugin, ProjectTrack};
use resonance_app::state::{InstrumentIcon, InstrumentType};
use resonance_app::update::project_io::replay_diff::{
    id_set_eq, midi_notes_equal, structurally_compatible,
};
use resonance_audio::types::MidiNote;

fn empty_file() -> ProjectFile {
    ProjectFile::default()
}

fn track(id: u64, vol: f32) -> ProjectTrack {
    ProjectTrack {
        id,
        name: format!("T{id}"),
        order: id as usize,
        volume: vol,
        pan: 0.0,
        muted: false,
        soloed: false,
        fx_bypassed: false,
        record_armed: false,
        monitor_enabled: false,
        playback_source: resonance_common::PlaybackSource::Live,
        mono: true,
        input_device_name: None,
        input_port_index: Some(0),
        plugins: Vec::new(),
        track_type: "audio".to_string(),
        output_bus: None,
        instrument_type: InstrumentType::default(),
        instrument_icon: InstrumentIcon::default(),
        role: None,
        sub_track: None,
        midi_input_device: None,
        midi_input_channel: None,
        midi_output_device: None,
        midi_output_channel: None,
        freeze: resonance_common::TrackFreezeState::unfrozen(),
        external_instrument: None,
    }
}

fn plugin(id: u64) -> ProjectPlugin {
    ProjectPlugin {
        instance_id: id,
        bypassed: false,
        plugin_name: format!("P{id}"),
        clap_plugin_id: "com.example.foo".to_string(),
        clap_file_path: "/x/foo.clap".to_string(),
        state_file: format!("plugins/plugin_{id}.bin"),
        params: Vec::new(),
    }
}

#[test]
fn empty_projects_are_structurally_compatible() {
    let a = empty_file();
    let b = empty_file();
    assert!(structurally_compatible(&a, &b));
}

#[test]
fn scalar_only_track_diff_is_compatible() {
    let mut a = empty_file();
    let mut b = empty_file();
    a.tracks = vec![track(1, 0.0)];
    b.tracks = vec![track(1, -6.0)];
    assert!(structurally_compatible(&a, &b));
}

/// Tracks are not part of the shape (A-13i): the diff arms add a track
/// `a` lacks, remove one `b` lacks, and treat a renumbered track or a type
/// change as a remove + add (`entities::kept_tracks`).
#[test]
fn added_removed_renumbered_and_retyped_tracks_are_compatible() {
    let pairs = [
        (vec![track(1, 0.0)], vec![track(1, 0.0), track(2, 0.0)]),
        (vec![track(1, 0.0), track(2, 0.0)], vec![track(1, 0.0)]),
        (vec![track(1, 0.0)], vec![track(2, 0.0)]),
        (vec![track(1, 0.0)], {
            let mut t = track(1, 0.0);
            t.track_type = "instrument".to_string();
            vec![t]
        }),
    ];
    for (ta, tb) in pairs {
        let mut a = empty_file();
        let mut b = empty_file();
        a.tracks = ta;
        b.tracks = tb;
        assert!(structurally_compatible(&a, &b));
    }
}

/// Plugin chains are not part of the shape (A-13h): the diff arms add,
/// remove, reorder and re-instantiate (an identity change under the same
/// id) plugin instances one at a time.
#[test]
fn added_plugin_is_compatible() {
    let mut a = empty_file();
    let mut b = empty_file();
    a.tracks = vec![track(1, 0.0)];
    let mut t = track(1, 0.0);
    t.plugins = vec![plugin(10)];
    b.tracks = vec![t];
    assert!(structurally_compatible(&a, &b));
    assert!(structurally_compatible(&b, &a), "and removed");
    a.master_plugins = vec![plugin(11)];
    assert!(structurally_compatible(&a, &b), "and on the master");
}

#[test]
fn plugin_reorder_is_compatible() {
    let mut a = empty_file();
    let mut b = empty_file();
    let mut t_a = track(1, 0.0);
    t_a.plugins = vec![plugin(10), plugin(11)];
    let mut t_b = track(1, 0.0);
    t_b.plugins = vec![plugin(11), plugin(10)];
    a.tracks = vec![t_a];
    b.tracks = vec![t_b];
    assert!(structurally_compatible(&a, &b));
}

#[test]
fn plugin_clap_identity_change_is_compatible() {
    let mut a = empty_file();
    let mut b = empty_file();
    let mut t_a = track(1, 0.0);
    t_a.plugins = vec![plugin(10)];
    let mut p = plugin(10);
    p.clap_plugin_id = "com.example.bar".to_string();
    let mut t_b = track(1, 0.0);
    t_b.plugins = vec![p];
    a.tracks = vec![t_a];
    b.tracks = vec![t_b];
    assert!(structurally_compatible(&a, &b));
}

#[test]
fn id_set_eq_ignores_order() {
    assert!(id_set_eq([1u64, 2, 3], [3u64, 2, 1]));
    assert!(!id_set_eq([1u64, 2], [1u64, 2, 3]));
}

#[test]
fn track_reorder_alone_is_compatible() {
    // Reorder via `.order` field — the actual track set is unchanged.
    let mut a = empty_file();
    let mut b = empty_file();
    a.tracks = vec![track(1, 0.0), track(2, 0.0)];
    let mut t1 = track(1, 0.0);
    t1.order = 1;
    let mut t2 = track(2, 0.0);
    t2.order = 0;
    b.tracks = vec![t2, t1];
    assert!(structurally_compatible(&a, &b));
}

#[test]
fn audio_file_path_change_is_compatible() {
    let mut a = empty_file();
    let mut b = empty_file();
    let mk = |id: u64, name: &str| ProjectClip {
        id,
        track_id: 1,
        start_sample: 0,
        name: name.into(),
        total_frames: 1000,
        trim_start_frames: 0,
        trim_end_frames: 0,
        audio_file: name.into(),
        asset_ref: None,
        fade_in_frames: 0,
        fade_in_curve: "equal_power".into(),
        fade_out_frames: 0,
        fade_out_curve: "equal_power".into(),
        gain_db: 0.0,
    };
    // Clips are not part of the shape (A-13i): a clip whose WAV changed is
    // deleted and reloaded under its id (`clips::kept_audio_clips`); an
    // added or removed clip is loaded or deleted.
    a.clips = vec![mk(1, "audio/a.wav")];
    b.clips = vec![mk(1, "audio/b.wav")];
    assert!(structurally_compatible(&a, &b));
    b.clips = vec![mk(1, "audio/a.wav"), mk(2, "audio/b.wav")];
    assert!(structurally_compatible(&a, &b));
    b.clips.clear();
    assert!(structurally_compatible(&a, &b));
}

#[test]
fn midi_notes_equal_field_wise() {
    let n = |note, vel, start, dur| MidiNote {
        note,
        velocity: vel,
        start_tick: start,
        duration_ticks: dur,
    };
    assert!(midi_notes_equal(&[], &[]));
    assert!(midi_notes_equal(
        &[n(60, 0.8, 0, 480)],
        &[n(60, 0.8, 0, 480)]
    ));
    assert!(!midi_notes_equal(
        &[n(60, 0.8, 0, 480)],
        &[n(62, 0.8, 0, 480)]
    ));
    assert!(!midi_notes_equal(
        &[n(60, 0.8, 0, 480)],
        &[n(60, 0.8, 0, 481)]
    ));
    // Different lengths are unequal.
    assert!(!midi_notes_equal(&[n(60, 0.8, 0, 480)], &[]));
}

/// The legacy flat `drum_groups` list is not part of the shape (A-13g):
/// `build_project_file` always writes it empty, so two undo snapshots can
/// never differ in it, and `DrumPatterns` restores whatever a file carries
/// on every origin. Drum groups live inside `drum_patterns` now.
#[test]
fn legacy_drum_group_id_set_change_is_compatible() {
    let mut a = empty_file();
    let mut b = empty_file();
    let g = |id: u64| DrumGroup {
        id,
        name: format!("g{id}"),
        color: [0, 0, 0],
        grid: 4,
        cycle: 16,
        phase: 0,
        pads: Vec::new(),
        density: 0.0,
        swing: 0.0,
        accent: 0.0,
        humanize: 0.0,
        fills: 0.0,
        style: String::new(),
        seed: 0,
    };
    a.drum_groups = vec![g(1)];
    b.drum_groups = vec![g(1), g(2)];
    assert!(structurally_compatible(&a, &b));
}

fn bus(id: u64, plugins: Vec<ProjectPlugin>) -> ProjectBus {
    ProjectBus {
        id,
        name: format!("B{id}"),
        order: 0,
        volume: 0.0,
        pan: 0.0,
        muted: false,
        fx_bypassed: false,
        plugins,
        is_return: false,
    }
}

/// The bus set is not part of the shape (A-13h): the diff arms add a bus
/// `a` lacks (with its chain) and remove one `b` lacks.
#[test]
fn bus_set_change_is_compatible() {
    let mut a = empty_file();
    let mut b = empty_file();
    a.busses = vec![bus(100, vec![plugin(10)])];
    b.busses = vec![bus(101, vec![plugin(11)])];
    assert!(structurally_compatible(&a, &b));
    b.busses = vec![bus(100, vec![plugin(10)]), bus(101, Vec::new())];
    assert!(structurally_compatible(&a, &b));
    b.busses = vec![bus(100, vec![plugin(12), plugin(10)])];
    assert!(structurally_compatible(&a, &b), "a bus chain change too");
}
