//! A frozen track goes stale when its arrangement content changes (code
//! review UPD-05).
//!
//! The only freeze invalidation was the pre-dispatch gate on direct input
//! edits (notes, lyrics, plugin params). Compose regeneration, bar
//! insert/remove and tempo edits all reshape what the track would render,
//! but none of them passed that gate, so the engine kept playing the old
//! frozen render and the UI offered no refreeze. The app now compares each
//! frozen track's content with the baseline its cache was rendered from
//! after every dispatch.

use resonance_app::message::{ArrangementMessage, Message, TrackMessage, TransportMessage};
use resonance_app::state::FreezeStatus;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::generate::{self as generate_proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{KeyScale, Request, Response};
use crate::common::roundtrip;

const TRACK: u64 = 10;

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn progression(app: &mut Resonance, section_id: resonance_control::ids::SectionDefinitionId, numerals: &[&str]) {
    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(numerals.iter().map(|s| (*s).to_owned()).collect());
    call(app, "harmony.apply_progression", &params)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
}

/// A project whose track 10 carries a generated bass part in a 4-bar
/// section, frozen with that content.
fn frozen_generated_track() -> (Resonance, Receiver<AudioCommand>, resonance_control::ids::SectionDefinitionId) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/freeze-stale-content.rprj"));
    app.test_set_sample_rate(48_000);
    app.test_add_track(TRACK, TrackType::Instrument);

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
    progression(&mut app, section_id, &["i", "iv", "v", "i"]);
    let generated: GenerateResult = call(
        &mut app,
        "generate.part",
        &generate_proto::PartParams {
            section_id,
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
    echo_clip_loads(&mut app, &rx);

    app.test_set_freeze_status(
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
    (app, rx, section_id)
}

/// Play the engine's part: echo every `LoadMidiClipDirect` it was sent
/// back as `MidiClipCreated`, then let a Tick run the post-dispatch checks.
fn echo_clip_loads(app: &mut Resonance, rx: &Receiver<AudioCommand>) {
    let loads: Vec<AudioCommand> = rx.try_iter().collect();
    for cmd in loads {
        if let AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } = cmd
        {
            app.test_apply_engine_event(AudioEvent::MidiClipCreated {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                name,
                notes,
                trim_start_ticks,
                trim_end_ticks,
            });
        }
    }
    let _ = app.update(Message::Tick);
}

fn is_frozen(app: &Resonance) -> bool {
    matches!(app.test_freeze_status(TRACK), FreezeStatus::Frozen { .. })
}

fn is_stale(app: &Resonance) -> bool {
    matches!(app.test_freeze_status(TRACK), FreezeStatus::Stale { .. })
}

#[test]
fn a_mixer_edit_leaves_the_track_frozen() {
    let (mut app, rx, _) = frozen_generated_track();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    echo_clip_loads(&mut app, &rx);
    assert!(is_frozen(&app), "volume is not a freeze input");
}

#[test]
fn a_rename_that_rederives_the_same_notes_leaves_the_track_frozen() {
    let (mut app, rx, _) = frozen_generated_track();
    // The derived clip's name embeds the track name, so a rename tears the
    // clip down and re-installs it — with the same notes.
    let _ = app.update(Message::Track(TrackMessage::SetTrackName(TRACK, "Low end".into())));
    assert!(is_frozen(&app), "the re-derived clip's echo is still pending");
    echo_clip_loads(&mut app, &rx);
    assert!(is_frozen(&app), "same content, still a valid freeze");
}

#[test]
fn a_chord_change_that_regenerates_the_part_marks_it_stale() {
    let (mut app, rx, section_id) = frozen_generated_track();
    progression(&mut app, section_id, &["i", "VI", "III", "VII"]);
    echo_clip_loads(&mut app, &rx);
    assert!(is_stale(&app), "the frozen render no longer matches the part");
}

#[test]
fn inserting_bars_before_the_part_marks_it_stale() {
    let (mut app, rx, _) = frozen_generated_track();
    let _ = app.update(Message::Arrangement(ArrangementMessage::InsertBars {
        at_bar: 1,
        count: 2,
    }));
    echo_clip_loads(&mut app, &rx);
    assert!(is_stale(&app), "the part moved in time");
}

#[test]
fn a_tempo_change_marks_it_stale() {
    let (mut app, rx, _) = frozen_generated_track();
    let _ = app.update(Message::Transport(TransportMessage::SetBpmText("140".into())));
    let _ = app.update(Message::Transport(TransportMessage::CommitBpm));
    echo_clip_loads(&mut app, &rx);
    assert!(is_stale(&app), "the notes now play at another tempo");
}

/// A structural undo (the full `ClearAll` replay) back to a state where
/// the track is frozen must keep the content baseline its cache was
/// rendered from. `replay_loaded_project`'s `freeze.reset()` used to wipe
/// every baseline, so the restored `Frozen` track never went stale again
/// until its next freeze (FU-H2b).
#[test]
fn a_full_replay_undo_keeps_the_frozen_content_baseline() {
    let (mut app, rx, _) = frozen_generated_track();
    // A real freeze directory, so the restore keeps the track `Frozen`
    // rather than downgrading it for a missing cache file.
    let root = tempfile::tempdir().expect("temp dir");
    let project = root.path().join("song.rproj");
    std::fs::create_dir_all(&project).expect("project dir");
    let freeze_dir = project.with_extension("freeze");
    std::fs::create_dir_all(&freeze_dir).expect("freeze dir");
    std::fs::write(freeze_dir.join("freeze_10.wav"), b"").expect("cache file");
    app.test_set_project_path(project);

    let snapshot = app.test_snapshot_for_undo();
    app.test_add_track(99, TrackType::Audio);
    let _ = rx.try_iter().count();
    app.test_begin_restore_from_snapshot(snapshot);
    assert!(
        rx.try_iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a structural undo takes the full replay"
    );
    app.test_apply_engine_event(AudioEvent::AllCleared);
    echo_clip_loads(&mut app, &rx);
    assert!(is_frozen(&app), "the restore brings the freeze back");

    let _ = app.update(Message::Transport(TransportMessage::SetBpmText("140".into())));
    let _ = app.update(Message::Transport(TransportMessage::CommitBpm));
    echo_clip_loads(&mut app, &rx);
    assert!(
        is_stale(&app),
        "a content change after the restore still invalidates the freeze"
    );
}

// ---- plugin automation (code review ENG-08) ------------------------------

/// The freeze bakes the track's plugin-param automation, so editing one of
/// those lanes changes what the track would render.
mod plugin_automation {
    use super::*;
    use resonance_app::state::PluginSlotState;
    use resonance_audio::types::ParamInfo;
    use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

    const SYNTH_TRACK: u64 = 20;
    const INSTANCE: u64 = 7_001;

    fn frozen_synth_track() -> Resonance {
        let (mut app, _task, _rx) = Resonance::new_for_test_with_capture();
        app.test_add_track(SYNTH_TRACK, TrackType::Instrument);
        app.test_push_track_plugin(
            SYNTH_TRACK,
            PluginSlotState::new(
                INSTANCE,
                "Test Synth".to_string(),
                "com.test.synth".to_string(),
                "/plugins/test.clap".to_string(),
                vec![ParamInfo {
                    id: 3,
                    name: "Cutoff".to_string(),
                    min_value: 0.0,
                    max_value: 1.0,
                    ..Default::default()
                }],
                false,
            ),
        );
        app.test_set_freeze_status(
            SYNTH_TRACK,
            FreezeStatus::Frozen {
                cache_ref: FreezeCacheRef::new(
                    "freeze_20.wav".to_string(),
                    48_000,
                    32,
                    0,
                    FreezeCacheStatus::Frozen,
                ),
            },
        );
        app
    }

    fn lane_on(app: &mut Resonance, target: AutomationTarget) {
        let lane = AutomationLane::new(
            1,
            target,
            vec![
                Breakpoint::new(0, 0.0, CurveKind::Linear),
                Breakpoint::new(48_000, 1.0, CurveKind::Linear),
            ],
        );
        app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane });
        let _ = app.update(Message::Tick);
    }

    fn status(app: &Resonance) -> FreezeStatus {
        app.test_freeze_status(SYNTH_TRACK)
    }

    #[test]
    fn a_plugin_param_lane_on_the_track_marks_it_stale() {
        let mut app = frozen_synth_track();
        lane_on(
            &mut app,
            AutomationTarget::PluginParam {
                instance: INSTANCE,
                param_id: 3,
            },
        );
        assert!(
            matches!(status(&app), FreezeStatus::Stale { .. }),
            "the frozen render no longer has the track's automation"
        );
    }

    #[test]
    fn a_fader_lane_leaves_the_track_frozen() {
        let mut app = frozen_synth_track();
        // Gain automation applies live, after the frozen cache.
        lane_on(&mut app, AutomationTarget::TrackGain(SYNTH_TRACK));
        assert!(matches!(status(&app), FreezeStatus::Frozen { .. }));
    }
}
