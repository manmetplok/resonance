//! Re-rendering a vocal must never delete audio the saved project or the
//! undo history still points at (FU-C1a, data loss).
//!
//! On load, `rebuild_vocal_audio_clips` maps each vocal lane to the clip
//! the project file names — `audio/clip_<id>.wav`, the only name a save,
//! an autosave or an undo snapshot records. The re-render paths (the
//! tear-down when a render is queued, and the swap when it lands) then
//! `unlink`ed that path as if it were a superseded render, deleting the
//! file the saved `project.json` and every undo snapshot reference: undo
//! the re-render, or reopen the saved project, and the vocal was gone.
//!
//! Only a rendered take (`vocal_*.wav`) that no installed clip still
//! points at may be unlinked by a re-render; everything else is left for
//! the save-time reaper (FU-B3), which only ever touches takes.

use std::path::{Path, PathBuf};

use resonance_app::compose::messages::VocalAudioReadyData;
use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::project::{LoadedProject, ProjectClip, ProjectFile};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, TrackType};
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::section as section_proto;
use resonance_control::methods::vocal as proto;
use resonance_control::{Request, Response};

use crate::common::roundtrip;

const TRACK: u64 = 50;
/// The saved vocal clip's id — well inside the derived range, as a
/// rendered clip's would be.
const SAVED_CLIP: u64 = 900_001;

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn write_wav(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("audio dir");
    resonance_app::update::compose::vocal_audio_io::write_rendered_wav(
        path.parent().unwrap(),
        &[0.1f32; 64],
        48_000,
    )
    .and_then(|written| std::fs::rename(&written, path).map_err(|e| e.to_string()))
    .expect("write fixture WAV");
}

/// A saved project on disk: a vocal lane with a generated melody, whose
/// rendered audio the save stored as `audio/clip_<SAVED_CLIP>.wav`.
struct Saved {
    _root: tempfile::TempDir,
    project_dir: PathBuf,
    file: ProjectFile,
    definition: u64,
    placement: u64,
}

impl Saved {
    fn clip_wav(&self) -> PathBuf {
        self.project_dir.join(resonance_app::project::clip_audio_file(SAVED_CLIP))
    }

    fn loaded(&self) -> LoadedProject {
        LoadedProject {
            file: self.file.clone(),
            project_dir: self.project_dir.clone(),
            midi_notes: std::collections::HashMap::new(),
            plugin_states: std::collections::HashMap::new(),
        }
    }

    /// Open the saved project in a fresh app, exactly as `ProjectLoaded`
    /// would (the caller of `replay_loaded_project` sets the path).
    fn open(&self) -> Resonance {
        let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
        app.test_set_active_project(true);
        app.test_replay_loaded_project_from(self.loaded());
        app.test_set_project_path(self.project_dir.clone());
        app
    }
}

fn saved_project() -> Saved {
    let root = tempfile::tempdir().expect("temp dir");
    let project_dir = root.path().join("song.rproj");

    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_set_project_path(project_dir.clone());
    app.test_set_sample_rate(48_000);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(TRACK, TrackType::Vocal);

    let section_id = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    let definition = u64::from(section_id);
    app.test_install_vocal_lane(definition, TRACK);
    let mut params =
        resonance_control::methods::harmony::ApplyProgressionParams::for_section(section_id);
    params.key = Some(resonance_control::KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    call(&mut app, "harmony.apply_progression", &params)
        .result::<resonance_control::methods::harmony::ApplyProgressionResult>()
        .expect("progression applies");

    let (placement, start_bar) = {
        let p = &app.compose_state().placements[0];
        (p.id, p.start_bar)
    };
    let start_sample = app.test_tempo_map().bar_to_sample(start_bar);

    let mut file = app.test_build_project_file();
    file.clips.push(ProjectClip {
        id: SAVED_CLIP,
        track_id: TRACK,
        start_sample,
        name: "Verse · Vocal".to_owned(),
        total_frames: 64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        audio_file: resonance_app::project::clip_audio_file(SAVED_CLIP),
        asset_ref: None,
        fade_in_frames: 0,
        fade_in_curve: "equal_power".to_owned(),
        fade_out_frames: 0,
        fade_out_curve: "equal_power".to_owned(),
        gain_db: 0.0,
    });
    let saved = Saved {
        _root: root,
        project_dir,
        file,
        definition,
        placement,
    };
    write_wav(&saved.clip_wav());
    saved
}

/// Reopening the saved project hands the engine a WAV that exists — the
/// "does it still play" check.
fn assert_saved_project_still_plays(saved: &Saved) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    let rx = app.test_capture_engine();
    app.test_replay_loaded_project_from(saved.loaded());
    let paths: Vec<PathBuf> = rx
        .try_iter()
        .filter_map(|cmd| match cmd {
            AudioCommand::LoadClipFromWav { clip_id, path, .. } if clip_id == SAVED_CLIP => {
                Some(path)
            }
            _ => None,
        })
        .collect();
    assert_eq!(paths, vec![saved.clip_wav()], "the reopened project loads the saved vocal");
    assert!(
        hound::WavReader::open(&paths[0]).is_ok(),
        "the saved project's vocal WAV was deleted by the re-render: {}",
        paths[0].display()
    );
}

#[test]
fn regenerating_after_a_reload_keeps_the_saved_vocal_wav() {
    let saved = saved_project();
    let mut app = saved.open();
    assert_eq!(
        app.test_vocal_audio_clips(TRACK),
        vec![(saved.definition, SAVED_CLIP)],
        "precondition: the load claims the saved clip as the lane's audio"
    );

    // Re-roll + re-render the lane: the render request tears the old
    // clip down before the new take is queued.
    call(
        &mut app,
        "vocal.generate",
        &proto::GenerateParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(resonance_control::ids::SectionDefinitionId(saved.definition)),
            seed: Some(7),
            lyrics: true,
        },
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate succeeds");
    assert!(
        app.test_vocal_audio_clips(TRACK).is_empty(),
        "precondition: the re-render tore the loaded clip down"
    );

    assert!(saved.clip_wav().exists(), "re-render deleted {}", saved.clip_wav().display());
    assert_saved_project_still_plays(&saved);
}

#[test]
fn installing_a_render_over_a_reloaded_clip_keeps_the_saved_vocal_wav() {
    let saved = saved_project();
    let mut app = saved.open();
    app.test_set_vocal_render_epoch(saved.definition, TRACK, 1);
    let fresh = saved.project_dir.join("audio").join("vocal_1.wav");
    write_wav(&fresh);
    let start = app.test_tempo_map().bar_to_sample(
        app.compose_state().placements[0].start_bar,
    );

    let _ = app.update(Message::Compose(ComposeMessage::VocalAudioReady(Box::new(
        VocalAudioReadyData {
            definition_id: saved.definition,
            track_id: TRACK,
            wav_path: fresh.clone(),
            placements: vec![(saved.placement, start)],
            clip_name: "Verse · Vocal".to_owned(),
            trim_start_frames: 0,
            trim_end_frames: 0,
            lead_ticks: 0,
            render_epoch: 1,
            bpm: 120.0,
        },
    ))));
    let installed = app.test_vocal_audio_clips(TRACK);
    assert_eq!(installed.len(), 1, "the new render is installed");
    assert_ne!(installed[0].1, SAVED_CLIP, "…replacing the loaded clip");

    assert!(fresh.exists(), "the new take is the live clip's WAV");
    assert!(saved.clip_wav().exists(), "install deleted {}", saved.clip_wav().display());
    assert_saved_project_still_plays(&saved);
}

/// The cleanup a re-render is for still happens: a superseded take that
/// nothing else references is unlinked, one still shared by another
/// placement's clip is kept.
#[test]
fn a_superseded_take_is_unlinked_only_once_nothing_references_it() {
    let root = tempfile::tempdir().expect("temp dir");
    let project_dir = root.path().join("song.rproj");
    let audio = project_dir.join("audio");
    let shared = audio.join("vocal_10.wav");
    let fresh = audio.join("vocal_11.wav");
    write_wav(&shared);
    write_wav(&fresh);

    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_set_project_path(project_dir.clone());
    app.test_add_track(TRACK, TrackType::Vocal);
    let section_id = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    let definition = u64::from(section_id);
    app.test_install_vocal_lane(definition, TRACK);
    let placement = app.compose_state().placements[0].id;
    // One render, installed on two placements (the second one's placement
    // no longer exists, so this render will not replace it).
    app.test_install_vocal_audio_clip(definition, placement, TRACK, 900_010, shared.clone());
    app.test_install_vocal_audio_clip(definition, 777_777, TRACK, 900_011, shared.clone());
    app.test_set_vocal_render_epoch(definition, TRACK, 1);

    let ready = |wav: &Path| {
        Message::Compose(ComposeMessage::VocalAudioReady(Box::new(VocalAudioReadyData {
            definition_id: definition,
            track_id: TRACK,
            wav_path: wav.to_path_buf(),
            placements: vec![(placement, 0)],
            clip_name: "Verse · Vocal".to_owned(),
            trim_start_frames: 0,
            trim_end_frames: 0,
            lead_ticks: 0,
            render_epoch: 1,
            bpm: 120.0,
        })))
    };
    let _ = app.update(ready(&fresh));
    assert!(shared.exists(), "another installed clip still plays {}", shared.display());

    // The next swap supersedes `fresh`, which nothing else references.
    let newer = audio.join("vocal_12.wav");
    write_wav(&newer);
    let _ = app.update(ready(&newer));
    assert!(!fresh.exists(), "an unreferenced superseded take is reaped");
    assert!(newer.exists());
    assert!(shared.exists(), "left for the save-time reaper (FU-B3)");
}
