//! `vocal.render` must render the **whole** track, not its first lane
//! (ba doc #271 V2 follow-up).
//!
//! A vocal track carries one lane per section it sings in. `vocal.render`
//! with only a `track_id` resolved "the track's first vocal lane" — the
//! rule the lyric methods use, where one write lands on one lane — and
//! rendered that lane alone. The call returned `done` and `song.vocal`
//! reported `rendered`, while lanes 2..n kept whatever audio a previous
//! session had left them: on a track whose notes had just been rewritten,
//! every section but the first sang the old melody, and once the stale
//! WAVs went missing those sections saved out silent.
//!
//! These tests pin the fan-out at both observable points: every lane is
//! dispatched for re-render, and the job covering them only resolves once
//! all of them have landed.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, Request, Response};

const TRACK: u64 = 50;
const OTHER_TRACK: u64 = 51;

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/control-vocal-render-all-lanes.rprj",
    ));
    app
}

fn roundtrip(app: &mut Resonance, request: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// A section placed at `start_bar` with a vocal lane on `track`, its own
/// chord grid, and a generated melody + lyrics — i.e. a lane that has
/// everything it needs to render.
fn singing_lane(app: &mut Resonance, name: &str, start_bar: u32, track: u64) -> u64 {
    let response = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: name.to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    );
    let section_id = response
        .result::<section_proto::CreateResult>()
        .expect("section.create succeeds")
        .section_id;
    let _ = call(
        app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: section_id,
            start_bar,
        },
    );
    let def = u64::from(section_id);
    app.test_install_vocal_lane(def, track);

    let mut params =
        resonance_control::methods::harmony::ApplyProgressionParams::for_section(section_id);
    params.key = Some(resonance_control::KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    call(app, "harmony.apply_progression", &params)
        .result::<resonance_control::methods::harmony::ApplyProgressionResult>()
        .expect("progression applies");

    call(
        app,
        "vocal.generate",
        &proto::GenerateParams {
            track_id: ProtoTrackId(track),
            section_id: Some(section_id),
            seed: Some(7),
            lyrics: true,
        },
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate succeeds");

    def
}

/// One vocal track singing in three sections, each with a generated
/// melody, and each carrying audio from an earlier render.
fn three_lane_app() -> (Resonance, Vec<u64>) {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);
    let lanes = vec![
        singing_lane(&mut app, "Verse", 1, TRACK),
        singing_lane(&mut app, "Chorus", 5, TRACK),
        singing_lane(&mut app, "Outro", 9, TRACK),
    ];
    // Audio a previous session rendered, one clip per lane. A re-render
    // tears its lane's entry down; a lane that is never dispatched keeps
    // pointing at this stale clip — the exact symptom reported.
    for (i, def) in lanes.iter().enumerate() {
        let placement = app
            .test_placements()
            .into_iter()
            .find(|(_, d, _)| d == def)
            .expect("each lane's section is placed")
            .0;
        app.test_install_vocal_audio_clip(
            *def,
            placement,
            TRACK,
            900_100 + i as u64,
            std::path::PathBuf::from(format!("/tmp/nonexistent-stale-vocal-{i}.wav")),
        );
    }
    (app, lanes)
}

fn render(app: &mut Resonance, track_id: Option<u64>, section_id: Option<u64>) -> Response {
    call(
        app,
        "vocal.render",
        &proto::RenderParams {
            track_id: track_id.map(ProtoTrackId),
            section_id: section_id.map(SectionDefinitionId),
            voicebank: None,
        },
    )
}

fn job_state(app: &mut Resonance, job_id: u64) -> JobState {
    roundtrip(
        app,
        Request::new(99, "job.status", &serde_json::json!({ "job_id": job_id }))
            .expect("params serialize"),
    )
    .result::<JobStatus>()
    .expect("job.status succeeds")
    .state
}

/// The bug: only the first lane was re-rendered. Every lane on the track
/// must be dispatched, and every lane's previous audio replaced.
#[test]
fn a_track_level_render_reaches_every_lane() {
    let (mut app, lanes) = three_lane_app();

    let before: Vec<u64> = lanes
        .iter()
        .map(|def| {
            app.test_vocal_render_epoch(*def, TRACK)
                .expect("generate queued a render for each lane")
        })
        .collect();
    assert_eq!(
        app.test_vocal_audio_clips(TRACK).len(),
        lanes.len(),
        "each lane starts out holding a previous render's audio"
    );

    render(&mut app, Some(TRACK), None)
        .result::<JobStarted>()
        .expect("a track-level render starts a job");

    for (def, was) in lanes.iter().zip(&before) {
        let now = app
            .test_vocal_render_epoch(*def, TRACK)
            .expect("every lane still has a render epoch");
        assert!(
            now > *was,
            "lane {def} was not re-rendered (epoch {was} -> {now}); \
             a track-level render must reach every lane, not just the first"
        );
    }
    assert!(
        app.test_vocal_audio_clips(TRACK).is_empty(),
        "every lane's previous audio is torn down before the new render lands; \
         a lane left here is one still pointing at stale audio: {:?}",
        app.test_vocal_audio_clips(TRACK)
    );
}

/// The job must not resolve on the first lane's audio — a client that
/// waits on it would otherwise read a half-rendered track back as `done`.
#[test]
fn the_job_resolves_only_when_every_lane_has_landed() {
    let (mut app, lanes) = three_lane_app();

    let job_id = u64::from(
        render(&mut app, Some(TRACK), None)
            .result::<JobStarted>()
            .expect("a track-level render starts a job")
            .job_id,
    );
    assert_eq!(job_state(&mut app, job_id), JobState::Pending);

    for def in &lanes[..lanes.len() - 1] {
        app.control_jobs().complete_vocal_lane(*def, TRACK, 0);
        assert_eq!(
            job_state(&mut app, job_id),
            JobState::Pending,
            "the job resolved before every lane had rendered"
        );
    }

    app.control_jobs()
        .complete_vocal_lane(lanes[lanes.len() - 1], TRACK, 0);
    assert_eq!(job_state(&mut app, job_id), JobState::Done);
}

/// An explicit `section_id` still addresses exactly one lane.
#[test]
fn a_named_section_renders_that_lane_alone() {
    let (mut app, lanes) = three_lane_app();
    let before: Vec<u64> = lanes
        .iter()
        .map(|def| app.test_vocal_render_epoch(*def, TRACK).unwrap_or(0))
        .collect();

    render(&mut app, Some(TRACK), Some(lanes[1]))
        .result::<JobStarted>()
        .expect("the named lane renders");

    assert!(app.test_vocal_render_epoch(lanes[1], TRACK).unwrap() > before[1]);
    assert_eq!(app.test_vocal_render_epoch(lanes[0], TRACK).unwrap(), before[0]);
    assert_eq!(app.test_vocal_render_epoch(lanes[2], TRACK).unwrap(), before[2]);
}

/// Omitting `track_id` means every vocal *track*, not the first one.
#[test]
fn omitting_the_track_renders_every_vocal_track() {
    let (mut app, lanes) = three_lane_app();
    app.test_add_track(OTHER_TRACK, TrackType::Vocal);
    let other = singing_lane(&mut app, "Harmony", 13, OTHER_TRACK);

    let before: Vec<u64> = lanes
        .iter()
        .chain(std::iter::once(&other))
        .map(|def| {
            let track = if *def == other { OTHER_TRACK } else { TRACK };
            app.test_vocal_render_epoch(*def, track).unwrap_or(0)
        })
        .collect();

    render(&mut app, None, None)
        .result::<JobStarted>()
        .expect("a project-wide render starts a job");

    for (i, def) in lanes.iter().chain(std::iter::once(&other)).enumerate() {
        let track = if *def == other { OTHER_TRACK } else { TRACK };
        assert!(
            app.test_vocal_render_epoch(*def, track).unwrap() > before[i],
            "lane {def} on track {track} was not re-rendered"
        );
    }
}

/// A lane that cannot render (no notes) is skipped rather than failing
/// the batch — but a batch where nothing can render is still an error.
#[test]
fn an_unrenderable_lane_does_not_block_the_rest() {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);

    // An empty lane placed *first*, so the old first-lane resolution
    // would have failed the whole call.
    let empty = {
        let response = call(
            &mut app,
            "section.create",
            &section_proto::CreateParams {
                name: "Intro".to_owned(),
                length_bars: 4,
                scale: None,
                place: false,
            },
        );
        let section_id = response
            .result::<section_proto::CreateResult>()
            .expect("section.create succeeds")
            .section_id;
        let _ = call(
            &mut app,
            "section.place",
            &section_proto::PlaceParams {
                definition_id: section_id,
                start_bar: 1,
            },
        );
        let def = u64::from(section_id);
        app.test_install_vocal_lane(def, TRACK);
        def
    };
    let singing = singing_lane(&mut app, "Verse", 5, TRACK);

    render(&mut app, Some(TRACK), None)
        .result::<JobStarted>()
        .expect("the renderable lane still renders");
    assert!(app.test_vocal_render_epoch(singing, TRACK).unwrap() >= 2);
    assert_eq!(
        app.test_vocal_render_epoch(empty, TRACK),
        None,
        "the empty lane was never dispatched"
    );

    // With only the empty lane, the precise per-lane error survives.
    let mut bare = app_with_project();
    bare.test_add_track(TRACK, TrackType::Vocal);
    let response = call(
        &mut bare,
        "section.create",
        &section_proto::CreateParams {
            name: "Intro".to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    );
    let section_id = response
        .result::<section_proto::CreateResult>()
        .expect("section.create succeeds")
        .section_id;
    let _ = call(
        &mut bare,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: section_id,
            start_bar: 1,
        },
    );
    bare.test_install_vocal_lane(u64::from(section_id), TRACK);

    let error = render(&mut bare, Some(TRACK), None)
        .error
        .expect("a track with nothing to render is an error");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("no notes"), "{}", error.message);
}
