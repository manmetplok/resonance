//! Per-track identity colour (mixer-cleanup.md §6): auto-assigned from
//! `theme::TRACK_PALETTE` on creation, inherited by sub-tracks, persisted
//! in the project, and given deterministically to tracks of a project
//! saved before the field existed.

use resonance_app::message::{Message, ProjectIoMessage, TrackMessage};
use resonance_app::project::{
    load_project, save_project, LoadedProject, ProjectFile, ProjectTrack,
};
use resonance_app::state::SubTrackLink;
use resonance_app::theme::{track_palette_color, TRACK_PALETTE};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{ChainOwner, AudioCommand, AudioEvent};

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// Add an audio track the way the engine echo mirrors one into the
/// registry (`TrackAdded`), which is where a new track gets its order and
/// so its palette colour.
fn add_track(app: &mut Resonance, rx: &Receiver<AudioCommand>) -> u64 {
    let _ = drain(rx);
    let id = 1000 + app.test_build_project_file().tracks.len() as u64;
    app.test_apply_engine_event(AudioEvent::TrackAdded { track_id: id });
    id
}

/// A hermetic app with an active project, so edits are applied.
fn fresh_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    (app, rx)
}

fn color_of(app: &Resonance, id: u64) -> [u8; 3] {
    app.test_build_project_file()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("track {id} in the project"))
        .color
        .expect("a saved track always carries its colour")
}

fn project_track(id: u64, order: usize, color: Option<[u8; 3]>) -> ProjectTrack {
    use resonance_app::state::{InstrumentIcon, InstrumentType};
    ProjectTrack {
        id,
        name: format!("Track {}", order + 1),
        order,
        volume: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        fx_bypassed: false,
        record_armed: false,
        monitor_enabled: false,
        playback_source: resonance_common::PlaybackSource::Live,
        mono: false,
        input_device_name: None,
        input_port_index: None,
        plugins: Vec::new(),
        track_type: "instrument".to_string(),
        output_bus: None,
        instrument_type: InstrumentType::Synth,
        instrument_icon: InstrumentIcon::Music,
        role: None,
        sub_track: None,
        midi_input_device: None,
        midi_input_channel: None,
        midi_output_device: None,
        midi_output_channel: None,
        external_instrument: None,
        freeze: resonance_common::TrackFreezeState::unfrozen(),
        color,
    }
}

/// Open `file` in a fresh app through the real `ProjectLoaded` →
/// `ClearAll` → `AllCleared` round-trip.
fn open(file: ProjectFile, dir: &std::path::Path) -> Resonance {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    let project = dir.join("project.rproj");
    app.test_set_project_path(project.clone());
    let loaded = LoadedProject {
        file,
        project_dir: project,
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    };
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(loaded),
    ))));
    assert!(drain(&rx).iter().any(|c| matches!(c, AudioCommand::ClearAll)));
    app.test_apply_engine_event(AudioEvent::AllCleared);
    app
}

#[test]
fn new_tracks_cycle_the_palette_by_order() {
    let (mut app, rx) = fresh_app();
    let n = TRACK_PALETTE.len() + 2;
    let ids: Vec<u64> = (0..n).map(|_| add_track(&mut app, &rx)).collect();
    let file = app.test_build_project_file();
    let colors: Vec<[u8; 3]> = ids
        .iter()
        .map(|id| {
            let t = file.tracks.iter().find(|t| t.id == *id).unwrap();
            assert_eq!(t.color, Some(track_palette_color(t.order)), "track {id}");
            t.color.unwrap()
        })
        .collect();
    // A full turn of the palette: every hue once, then it wraps.
    let first_turn: std::collections::HashSet<_> =
        colors[..TRACK_PALETTE.len()].iter().collect();
    assert_eq!(first_turn.len(), TRACK_PALETTE.len(), "{colors:?}");
    assert_eq!(colors[TRACK_PALETTE.len()], colors[0]);
    assert_eq!(colors[TRACK_PALETTE.len() + 1], colors[1]);
}

#[test]
fn a_sub_track_inherits_its_parents_colour() {
    let (mut app, rx) = fresh_app();
    // Shift the parent off palette slot 0 so inheriting differs from the
    // sub-track's own order pick.
    let _ = add_track(&mut app, &rx);
    let parent = add_track(&mut app, &rx);
    let custom = [0x12, 0x34, 0x56];
    let _ = app.update(Message::Track(TrackMessage::SetTrackColor(parent, custom)));
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Track(parent),
        instance_id: 77,
        plugin_name: "Multi".to_owned(),
        clap_plugin_id: "com.test.multi".to_owned(),
        clap_file_path: "/plugins/multi.clap".to_owned(),
        params: Vec::new(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 3,
        output_port_names: (0..3).map(|p| format!("Out {p}")).collect(),
    });
    let file = app.test_build_project_file();
    let subs: Vec<&ProjectTrack> = file
        .tracks
        .iter()
        .filter(|t| t.sub_track.is_some_and(|l| l.parent_track_id == parent))
        .collect();
    assert_eq!(subs.len(), 2);
    for s in subs {
        assert_eq!(s.color, Some(custom), "sub-track {}", s.id);
    }
}

#[test]
fn colour_survives_save_and_load() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, rx) = fresh_app();
    let a = add_track(&mut app, &rx);
    let b = add_track(&mut app, &rx);
    let custom = [0xab, 0xcd, 0xef];
    let _ = app.update(Message::Track(TrackMessage::SetTrackColor(b, custom)));
    let a_color = color_of(&app, a);

    let dir = tmp.path().join("saved.rproj");
    save_project(&dir, &app.test_build_project_file(), &[], &[]).expect("save");
    let loaded = load_project(&dir).expect("load");
    let reopened = open(loaded.file, tmp.path());
    assert_eq!(color_of(&reopened, a), a_color);
    assert_eq!(color_of(&reopened, b), custom);
}

#[test]
fn a_legacy_project_opens_with_deterministic_colours() {
    let tmp = tempfile::tempdir().unwrap();
    // As written by a build before the field existed: no `color` key.
    let mut file = ProjectFile {
        tracks: vec![
            project_track(1, 0, None),
            project_track(2, 1, None),
            project_track(3, 2, Some([1, 2, 3])),
            project_track(4, 3, None),
        ],
        ..ProjectFile::default()
    };
    file.tracks[3].sub_track = Some(SubTrackLink {
        parent_track_id: 2,
        output_port_index: 1,
    });
    let json = serde_json::to_value(&file).unwrap();
    assert!(
        json["tracks"][0].get("color").is_none(),
        "a legacy track has no colour key: {}",
        json["tracks"][0]
    );
    let legacy: ProjectFile = serde_json::from_value(json).unwrap();

    let first = open(legacy.clone(), tmp.path());
    let second = open(legacy, tmp.path());
    for app in [&first, &second] {
        assert_eq!(color_of(app, 1), track_palette_color(0));
        assert_eq!(color_of(app, 2), track_palette_color(1));
        // A saved colour wins over the order pick.
        assert_eq!(color_of(app, 3), [1, 2, 3]);
        // A legacy sub-track takes its parent's colour.
        assert_eq!(color_of(app, 4), track_palette_color(1));
    }
}

/// A sub-track's saved colour is redundant with its parent's: a file
/// where the two disagree (hand-edited), or that lists the sub-track
/// before its parent, still opens with the cluster in the parent's
/// colour — the load does not depend on the file being consistent or
/// ordered.
#[test]
fn a_sub_track_opens_in_its_parents_colour_whatever_the_file_says() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = [0x10, 0x80, 0x40];
    let mut sub = project_track(7, 0, Some([0xff, 0, 0]));
    sub.sub_track = Some(SubTrackLink {
        parent_track_id: 2,
        output_port_index: 1,
    });
    let mut legacy_sub = project_track(8, 1, None);
    legacy_sub.sub_track = Some(SubTrackLink {
        parent_track_id: 2,
        output_port_index: 2,
    });
    let file = ProjectFile {
        tracks: vec![sub, legacy_sub, project_track(2, 2, Some(parent))],
        ..ProjectFile::default()
    };
    let app = open(file, tmp.path());
    assert_eq!(color_of(&app, 2), parent);
    assert_eq!(color_of(&app, 7), parent, "a disagreeing saved colour");
    assert_eq!(color_of(&app, 8), parent, "a legacy sub-track listed first");
}

/// The read model reports a sub-track in its parent's colour too — what
/// its strip draws — even for a sub-track whose own stored colour was
/// never synced (built outside every creation path).
#[test]
fn song_tracks_reports_a_sub_track_in_its_parents_colour() {
    use resonance_app::state::{SubTrackLink, TrackState};
    let (mut app, rx) = fresh_app();
    let _ = add_track(&mut app, &rx);
    let parent = add_track(&mut app, &rx);
    let custom = [0x0a, 0x0b, 0x0c];
    let _ = app.update(Message::Track(TrackMessage::SetTrackColor(parent, custom)));
    let mut stray = TrackState::new_instrument(900, 5);
    stray.sub_track = Some(SubTrackLink {
        parent_track_id: parent,
        output_port_index: 1,
    });
    stray.color = [1, 1, 1];
    app.test_registry_mut().tracks.push(stray);

    let summary: resonance_control::methods::song::SongSummary =
        crate::common::call(&mut app, "song.summary", serde_json::json!({}))
            .result()
            .expect("song.summary succeeds");
    let color_of_wire = |id: u64| {
        summary
            .tracks
            .iter()
            .find(|t| t.id.0 == id)
            .unwrap_or_else(|| panic!("track {id} in the summary"))
            .color
            .clone()
    };
    assert_eq!(color_of_wire(parent).as_deref(), Some("#0a0b0c"));
    assert_eq!(color_of_wire(900).as_deref(), Some("#0a0b0c"));
}

/// A bus rename persists: saved, reopened, the bus has its new name.
#[test]
fn a_bus_rename_survives_save_and_load() {
    use resonance_app::message::BusMessage;
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, _rx) = fresh_app();
    app.test_add_bus(1, "Verb");
    let _ = app.update(Message::Bus(BusMessage::RenameBus(1, "Plate".into())));

    let dir = tmp.path().join("saved.rproj");
    save_project(&dir, &app.test_build_project_file(), &[], &[]).expect("save");
    let loaded = load_project(&dir).expect("load");
    assert_eq!(loaded.file.busses.first().map(|b| b.name.as_str()), Some("Plate"));
    let reopened = open(loaded.file, tmp.path());
    assert_eq!(
        reopened.test_registry().busses.first().map(|b| b.name.as_str()),
        Some("Plate")
    );
}
