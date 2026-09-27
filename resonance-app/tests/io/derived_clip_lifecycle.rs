//! The compose derived-clip map and id counter across user deletes and
//! project reopens (code review FU-A6b, FU-A6c; `docs/design/A-6-derived-clips.md` §8).
//!
//! FU-A6b: deleting a generated clip on the timeline used to leave its
//! `(section, placement, track) → clip` entry behind. The dangling entry
//! was written into every later snapshot, survived a diff-path undo, kept
//! the UPD-05 freeze check skipping the track ("echo pending" forever) and
//! made a section resize re-generate the clip the user had deleted. A user
//! delete now drops the entry; undoing the delete brings it back with the
//! clip.
//!
//! FU-A6c: the derived-id counter is reset on every disk load and only
//! reserved past the clips in the file, so after a reopen it re-issued the
//! id of a vocal render deleted before the save, whose
//! `audio/clip_<id>.wav` a backup (or the file of an older undo state) can
//! still name, and the next render overwrote it. The engine's STATE-08
//! scan skips the derived range since FU-A6a (the app owns it), so the app
//! scans for its own range when it learns the project dir.
//!
//! FU-A6d: `compose.vocal_audio.clips` (the rendered-vocal-audio-clip map,
//! keyed the same way as `derived_clips` but out of A-6's scope — see
//! `docs/design/A-6-derived-clips.md` §1) has the same "clip the user moved
//! outlives its placement's purge" hole `remove_bars` had for MIDI derived
//! clips before FU-A6b, but only in `remove_bars`: a single-clip user
//! delete (`clip.delete` / the GUI Delete key) and a whole-placement delete
//! already scrub it (`engine_events::clips::deleted`,
//! `purge_placement_outputs`).

use std::collections::HashMap;
use std::path::Path;

use resonance_app::compose::messages::VocalAudioReadyData;
use resonance_app::compose::ComposeMessage;
use resonance_app::message::{
    ClipMessage, Message, MidiClipMessage, ProjectIoMessage, TrackMessage, TransportMessage,
};
use resonance_app::state::ids::DERIVED_CLIP_ID_BASE;
use resonance_app::state::FreezeStatus;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ClipId, TrackId, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::arrangement::RemoveBarsParams;
use resonance_control::methods::generate::{self as generate_proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{KeyScale, Request, Response};

use crate::common::roundtrip;

const TRACK: TrackId = 10;

type DerivedMap = HashMap<(u64, u64, TrackId), ClipId>;

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// Play the engine's part for every clip command sent so far: a
/// `LoadMidiClipDirect` echoes `MidiClipCreated`, a `DeleteMidiClip`
/// echoes `MidiClipDeleted`, in send order. Then a Tick.
fn echo(app: &mut Resonance, rx: &Receiver<AudioCommand>) {
    let cmds: Vec<AudioCommand> = rx.try_iter().collect();
    echo_sent(app, cmds);
}

fn echo_sent(app: &mut Resonance, cmds: Vec<AudioCommand>) {
    for cmd in &cmds {
        match cmd.clone() {
            AudioCommand::LoadMidiClipDirect {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                notes,
                name,
                trim_start_ticks,
                trim_end_ticks,
            } => app.test_apply_engine_event(AudioEvent::MidiClipCreated {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                name,
                notes,
                trim_start_ticks,
                trim_end_ticks,
            }),
            AudioCommand::DeleteMidiClip { clip_id } => {
                app.test_apply_engine_event(AudioEvent::MidiClipDeleted { clip_id })
            }
            _ => {}
        }
    }
    let _ = app.update(Message::Tick);
}

struct Song {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    section: SectionDefinitionId,
    clip: ClipId,
    _root: tempfile::TempDir,
}

/// Track 10 with a generated bass part in a 4-bar section, in a saved
/// project (so edits record undo steps).
fn generated_part() -> Song {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    let root = tempfile::tempdir().expect("temp dir");
    let project = root.path().join("song.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("project dir");
    std::fs::create_dir_all(project.with_extension("freeze")).expect("freeze dir");
    app.test_set_active_project(true);
    app.test_set_project_path(project);
    app.test_set_sample_rate(48_000);
    app.test_add_track(TRACK, TrackType::Instrument);

    let section = call(
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
    let mut params = harmony_proto::ApplyProgressionParams::for_section(section);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].iter().map(|s| (*s).to_owned()).collect());
    call(&mut app, "harmony.apply_progression", &params)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
    let generated: GenerateResult = call(
        &mut app,
        "generate.part",
        &generate_proto::PartParams {
            section_id: section,
            track_id: ProtoTrackId(TRACK),
            role: GenerateRole::Bass,
            chord_count: None,
            beats_per_chord: None,
            sevenths: None,
            seed: Some(42),
            options: None,
        },
    )
    .result()
    .expect("generate.part succeeds");
    assert_eq!(generated.clip_ids.len(), 1);
    let clip = generated.clip_ids[0].0;
    echo(&mut app, &rx);
    assert!(
        derived(&app).values().any(|id| *id == clip),
        "the generated clip is in the derived map"
    );
    Song {
        app,
        rx,
        section,
        clip,
        _root: root,
    }
}

fn derived(app: &Resonance) -> DerivedMap {
    app.compose_state().derived_clips.clone()
}

fn has_clip(app: &Resonance, id: ClipId) -> bool {
    app.test_midi_clips().iter().any(|c| c.id == id)
}

fn clips_on_track(app: &Resonance) -> Vec<ClipId> {
    app.test_midi_clips()
        .iter()
        .filter(|c| c.track_id == TRACK)
        .map(|c| c.id)
        .collect()
}

/// The user deletes the generated clip on the timeline (Delete key).
fn user_deletes(s: &mut Song) {
    let _ = s
        .app
        .update(Message::MidiClip(MidiClipMessage::DeleteMidiClip(s.clip)));
    echo(&mut s.app, &s.rx);
    assert!(!has_clip(&s.app, s.clip), "the engine echo dropped the clip");
}

/// Run an undo to completion, whichever restore path it takes.
fn undo(s: &mut Song) {
    let _ = s.rx.try_iter().count();
    let _ = s.app.update(Message::Undo);
    let sent: Vec<AudioCommand> = s.rx.try_iter().collect();
    if sent.iter().any(|c| matches!(c, AudioCommand::ClearAll)) {
        s.app.test_apply_engine_event(AudioEvent::AllCleared);
    } else {
        echo_sent(&mut s.app, sent);
    }
    echo(&mut s.app, &s.rx);
}

// ---------------------------------------------------------------------------
// FU-A6b
// ---------------------------------------------------------------------------

#[test]
fn deleting_a_generated_clip_drops_its_derived_entry() {
    let mut s = generated_part();
    user_deletes(&mut s);
    assert!(
        !derived(&s.app).values().any(|id| *id == s.clip),
        "a deleted clip must not stay in the derived map: {:?}",
        derived(&s.app)
    );
    assert!(
        s.app
            .test_build_project_file()
            .derived_clips
            .unwrap_or_default()
            .iter()
            .all(|e| e.clip_id != s.clip),
        "nor in the file a save or an undo snapshot writes"
    );
}

#[test]
fn undoing_the_delete_restores_the_clip_and_its_entry() {
    let mut s = generated_part();
    let before = derived(&s.app);
    user_deletes(&mut s);
    undo(&mut s);
    assert!(has_clip(&s.app, s.clip), "undo brings the clip back");
    assert_eq!(derived(&s.app), before, "and its derived entry");
}

/// A diff-path undo of a later, unrelated edit restores the snapshot
/// taken after the delete — it must not carry a dangling entry either.
#[test]
fn a_fast_path_undo_after_the_delete_leaves_no_dangling_entry() {
    let mut s = generated_part();
    user_deletes(&mut s);
    let _ = s
        .app
        .update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    echo(&mut s.app, &s.rx);
    undo(&mut s);
    let dangling: Vec<ClipId> = derived(&s.app)
        .values()
        .copied()
        .filter(|id| !has_clip(&s.app, *id))
        .collect();
    assert!(dangling.is_empty(), "entries without a clip: {dangling:?}");
}

/// UPD-05 skips a frozen track while a derived entry on it points at an
/// unmirrored clip (an echo it takes to be pending). A deleted clip's
/// entry used to hold that forever, so the track never went stale again.
#[test]
fn a_frozen_track_whose_generated_clip_was_deleted_still_goes_stale() {
    let mut s = generated_part();
    user_deletes(&mut s);
    s.app.test_set_freeze_status(
        TRACK,
        FreezeStatus::Frozen {
            cache_ref: FreezeCacheRef::new(
                "freeze_10.wav".to_string(),
                48_000,
                32,
                0,
                FreezeCacheStatus::Frozen,
            ),
        },
    );
    // Any content change: a hand-drawn clip on the frozen track.
    s.app.test_push_midi_clip(resonance_app::state::MidiClipState {
        id: 77,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 960,
        name: "drawn".into(),
        notes: vec![resonance_audio::types::MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }]
        .into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    let _ = s
        .app
        .update(Message::Transport(TransportMessage::SetBpmText("140".into())));
    let _ = s.app.update(Message::Transport(TransportMessage::CommitBpm));
    echo(&mut s.app, &s.rx);
    assert!(
        matches!(s.app.test_freeze_status(TRACK), FreezeStatus::Stale { .. }),
        "the content changed; the freeze must go stale, got {:?}",
        s.app.test_freeze_status(TRACK)
    );
}

/// A resize re-derives only the lanes that have an entry ("a resize never
/// generates a lane the user did not ask for"). A lane whose clip the user
/// deleted is such a lane: the resize must not bring the clip back.
#[test]
fn a_section_resize_does_not_resurrect_a_deleted_generated_clip() {
    let mut s = generated_part();
    user_deletes(&mut s);
    let _ = s.app.update(Message::Compose(ComposeMessage::ResizeSection {
        definition_id: s.section.0,
        length_bars: 8,
    }));
    echo(&mut s.app, &s.rx);
    assert_eq!(
        clips_on_track(&s.app),
        Vec::<ClipId>::new(),
        "the resize re-created the clip the user deleted"
    );
}

/// Explicitly regenerating the lane still works after a delete, and
/// allocates a fresh id rather than the deleted one's.
#[test]
fn regenerating_after_a_delete_installs_a_new_clip() {
    let mut s = generated_part();
    user_deletes(&mut s);
    let generated: GenerateResult = call(
        &mut s.app,
        "generate.part",
        &generate_proto::PartParams {
            section_id: s.section,
            track_id: ProtoTrackId(TRACK),
            role: GenerateRole::Bass,
            chord_count: None,
            beats_per_chord: None,
            sevenths: None,
            seed: Some(42),
            options: None,
        },
    )
    .result()
    .expect("generate.part succeeds");
    echo(&mut s.app, &s.rx);
    let id = generated.clip_ids[0].0;
    assert!(has_clip(&s.app, id), "the regenerated clip is on the timeline");
    assert_ne!(id, s.clip, "a deleted clip's id is not handed out again");
    assert_eq!(clips_on_track(&s.app), vec![id]);
}

// ---------------------------------------------------------------------------
// FU-A6c
// ---------------------------------------------------------------------------

fn write_wav_named(dir: &Path, id: ClipId) {
    std::fs::create_dir_all(dir.join("audio")).expect("audio dir");
    crate::common::write_freeze_cache_wav(&dir.join("audio").join(format!("clip_{id}.wav")));
}

/// A derived-range `clip_<id>.wav` in the bundle whose clip is no longer
/// in the saved file (a deleted vocal render a backup still names): after
/// the reopen, a newly generated clip must not get that id.
#[test]
fn a_reopen_reserves_past_derived_clip_wavs_on_disk() {
    let s = generated_part();
    let file = s.app.test_build_project_file();
    let midi: Vec<(ClipId, Vec<resonance_audio::types::MidiNote>)> = s
        .app
        .test_midi_clips()
        .iter()
        .map(|mc| (mc.id, mc.notes.as_ref().clone()))
        .collect();
    let dir = s._root.path().join("reopened.rproj");
    std::fs::create_dir_all(dir.join("audio")).expect("bundle dir");
    resonance_app::project::save_project(&dir, &file, &[], &midi).expect("save");
    let orphan = DERIVED_CLIP_ID_BASE + 50;
    write_wav_named(&dir, orphan);
    // Below the range: the engine's scan owns those, the app ignores them.
    write_wav_named(&dir, 12);

    let loaded = resonance_app::project::load_project(&dir).expect("load");
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_replay_loaded_project_from(loaded);
    let _ = rx.try_iter().count();
    let next = app.compose_state().next_derived_clip_id;
    assert!(
        next > orphan,
        "after the reopen the counter ({next}) would re-issue {orphan}, whose WAV is on disk"
    );
}

/// The same for a Save As into a bundle that already holds derived-range
/// WAVs: from then on the session writes clip WAVs there.
#[test]
fn a_save_as_into_an_existing_bundle_reserves_past_its_derived_wavs() {
    let mut s = generated_part();
    let target = s._root.path().join("existing.rproj");
    let orphan = DERIVED_CLIP_ID_BASE + 500;
    write_wav_named(&target, orphan);
    let _ = s.app.update(Message::ProjectIo(ProjectIoMessage::SavePathSelected(Some(
        target.to_string_lossy().into_owned(),
    ))));
    let next = s.app.compose_state().next_derived_clip_id;
    assert!(
        next > orphan,
        "the counter ({next}) would re-issue {orphan}, whose WAV is in the new bundle"
    );
}

// ---------------------------------------------------------------------------
// FU-A6d
// ---------------------------------------------------------------------------

const VOCAL_TRACK: TrackId = 60;

/// An 8-bar vocal section, placed at bar 1, with a rendered vocal audio
/// clip installed on the lane (as `handle_vocal_audio_ready` would after a
/// real SVS render), in a saved project (so edits record undo steps).
/// Returns the installed clip's id alongside the app; the `TempDir` must
/// be kept alive by the caller for the project path to stay valid.
fn vocal_song_with_installed_audio()
-> (Resonance, Receiver<AudioCommand>, u64, u64, ClipId, tempfile::TempDir) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    let root = tempfile::tempdir().expect("temp dir");
    let project = root.path().join("song.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("project dir");
    std::fs::create_dir_all(project.with_extension("freeze")).expect("freeze dir");
    app.test_set_active_project(true);
    app.test_set_project_path(project);
    app.test_set_sample_rate(48_000);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(VOCAL_TRACK, TrackType::Vocal);

    let section = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 8,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    let definition = u64::from(section);
    app.test_install_vocal_lane(definition, VOCAL_TRACK);
    let (placement, start_bar) = {
        let p = &app.compose_state().placements[0];
        (p.id, p.start_bar)
    };
    let queued_start = app.test_tempo_map().bar_to_sample(start_bar);

    let _ = app.update(Message::Compose(ComposeMessage::VocalAudioReady(Box::new(
        VocalAudioReadyData {
            definition_id: definition,
            track_id: VOCAL_TRACK,
            wav_path: std::path::PathBuf::from("/tmp/fu-a6d-vocal.wav"),
            placements: vec![(placement, queued_start)],
            clip_name: "Verse · Vocal".to_owned(),
            trim_start_frames: 0,
            trim_end_frames: 0,
            lead_ticks: 0,
            render_epoch: 0,
            bpm: 120.0,
        },
    ))));
    let installed_clip = rx
        .try_iter()
        .find_map(|cmd| match cmd {
            AudioCommand::LoadClipFromWav {
                clip_id,
                track_id,
                start_sample,
                name,
                ..
            } => {
                // Mirror the engine's `ClipImported` echo, exactly as a
                // real render's `LoadClipFromWav` would get mirrored into
                // `r.clips` (`engine_events::clips::imported`).
                app.test_apply_engine_event(AudioEvent::ClipImported {
                    clip_id,
                    track_id,
                    start_sample,
                    duration_samples: 64,
                    name,
                    waveform_peaks: Vec::new(),
                });
                Some(clip_id)
            }
            _ => None,
        })
        .expect("the render installed an audio clip");
    assert_eq!(
        app.test_vocal_audio_clips(VOCAL_TRACK),
        vec![(definition, installed_clip)],
        "precondition: the rendered clip is in the vocal-audio map"
    );
    (app, rx, definition, placement, installed_clip, root)
}

/// `remove_bars` deleting a vocal audio clip the user dragged off its
/// placement's start bar (so the placement itself survives, and only the
/// audio-clip casualty path runs — not `purge_placement_outputs`) must not
/// leave the clip's `(definition, placement, track)` entry in
/// `vocal_audio.clips`. Mirrors FU-A6b's fix for `derived_clips`, which
/// covers this same `remove_bars` case for MIDI clips but never touched
/// the vocal-audio map.
#[test]
fn remove_bars_drops_a_moved_vocal_audio_clips_entry() {
    let (mut app, rx, definition, placement, clip_id, _root) = vocal_song_with_installed_audio();
    let start_bar = app
        .compose_state()
        .find_placement(placement)
        .expect("placement exists")
        .start_bar;

    // Drag the rendered clip two bars into the section, off the placement's
    // own start bar.
    let moved_to = app.test_tempo_map().bar_to_sample(start_bar + 2);
    let _ = app.update(Message::Clip(ClipMessage::MoveClipTo {
        clip_id,
        new_start_sample: moved_to,
        new_track_id: VOCAL_TRACK,
    }));
    assert!(
        app.test_clips().iter().any(|c| c.id == clip_id && c.start_sample == moved_to),
        "precondition: the clip moved"
    );
    let _ = rx.try_iter().count();

    // Remove exactly the bar the clip now sits on: 1-based `at_bar` is
    // `start_bar + 2 + 1`, distinct from the placement's own start bar
    // (`start_bar + 1`), so `removal_casualties` puts only the clip, not
    // the placement, in its casualty list.
    let response = call(
        &mut app,
        "arrangement.remove_bars",
        &RemoveBarsParams {
            at_bar: start_bar + 3,
            count: 1,
            confirm: true,
        },
    );
    assert!(response.error.is_none(), "remove_bars failed: {:?}", response.error);
    assert!(
        app.compose_state().find_placement(placement).is_some(),
        "precondition: the placement survives the removal"
    );
    assert!(
        !app.test_clips().iter().any(|c| c.id == clip_id),
        "precondition: the clip itself was removed"
    );

    assert!(
        app.test_vocal_audio_clips(VOCAL_TRACK).is_empty(),
        "a dangling vocal-audio entry survived remove_bars for a clip the user moved: {:?}",
        app.test_vocal_audio_clips(VOCAL_TRACK)
    );
    let _ = definition;
}

/// Known limitation (not fixed here, and not a regression from FU-A6d):
/// undoing a `remove_bars` that deleted a *moved* vocal audio clip brings
/// the clip's `ClipState` back, but not its `vocal_audio.clips` entry.
///
/// Unlike `derived_clips`, which A-6 made undo restore from the snapshot's
/// authoritative saved map, `vocal_audio.clips` is runtime-only and always
/// rebuilt positionally from `r.clips` on every reconcile (the
/// `VocalAudioClips` domain, `docs/design/A-6-derived-clips.md` §1: "out of
/// scope here"). That rebuild only claims a clip sitting exactly on a
/// placement's start bar, so a clip restored at the position the user
/// dragged it to is not reclaimed — the same gap the design doc already
/// calls out for a "derived clip the user moved" (§`Relation to
/// midi_clips`), just for the audio side. A lane in this state reads as
/// `not_rendered` until the user regenerates it; nothing crashes or
/// resurrects stale audio.
#[test]
fn undoing_the_removal_does_not_reclaim_a_moved_clips_vocal_audio_entry() {
    let (mut app, rx, _definition, placement, clip_id, _root) = vocal_song_with_installed_audio();
    let start_bar = app
        .compose_state()
        .find_placement(placement)
        .expect("placement exists")
        .start_bar;
    let moved_to = app.test_tempo_map().bar_to_sample(start_bar + 2);
    let _ = app.update(Message::Clip(ClipMessage::MoveClipTo {
        clip_id,
        new_start_sample: moved_to,
        new_track_id: VOCAL_TRACK,
    }));
    let _ = rx.try_iter().count();
    let response = call(
        &mut app,
        "arrangement.remove_bars",
        &RemoveBarsParams {
            at_bar: start_bar + 3,
            count: 1,
            confirm: true,
        },
    );
    assert!(response.error.is_none(), "remove_bars failed: {:?}", response.error);
    let _ = rx.try_iter().count();

    let sent: Vec<AudioCommand> = {
        let _ = app.update(Message::Undo);
        rx.try_iter().collect()
    };
    if sent.iter().any(|c| matches!(c, AudioCommand::ClearAll)) {
        app.test_apply_engine_event(AudioEvent::AllCleared);
    }
    let _ = rx.try_iter().count();

    assert!(
        app.test_clips().iter().any(|c| c.id == clip_id && c.start_sample == moved_to),
        "the clip itself comes back at its moved position"
    );
    assert!(
        app.test_vocal_audio_clips(VOCAL_TRACK).is_empty(),
        "documents the known gap: the positional rebuild does not reclaim a moved clip"
    );
}

