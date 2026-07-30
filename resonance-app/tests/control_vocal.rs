//! `vocal.*` control methods (ba doc #265, todo #1156): lyrics,
//! pronunciation overrides, and the SVS render job — driven through the
//! real `update()` path with synthesized requests.
//!
//! Vocal lanes are installed via `test_install_vocal_lane`; the SVS
//! model dir isn't present under test, so `vocal.render` stays a
//! `pending`/`running` job (the render falls back to MIDI-only) — the
//! tests assert the job is registered and that fast validation failures
//! fail the job synchronously, not that audio lands.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::job::{JobState, JobStarted, JobStatus, StatusParams};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, KeyScale, MutationAck, Request, Response};

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-vocal-test.rprj"));
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

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

/// A placed section with chords and a vocal lane on `track`, returning
/// (definition_id, proto track id).
fn vocal_section(app: &mut Resonance, track_raw: u64) -> (u64, ProtoTrackId) {
    let response = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    );
    let section_id = response
        .result::<section_proto::CreateResult>()
        .expect("section.create")
        .section_id;

    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(vec!["i".to_owned(), "iv".to_owned(), "v".to_owned(), "i".to_owned()]);
    let _ = call(app, "harmony.apply_progression", &params);

    app.test_add_track(track_raw, TrackType::Vocal);
    let def = u64::from(section_id);
    app.test_install_vocal_lane(def, track_raw);
    (def, ProtoTrackId(track_raw))
}

// ---------------- vocal.set_lyrics / set_line ----------------

#[test]
fn set_lyrics_replaces_the_draft() {
    let mut app = app_with_project();
    let (def, track) = vocal_section(&mut app, 30);

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: track,
            section_id: None,
            text: "hello world\nsecond line\nthird line".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_lyrics succeeds");

    assert_eq!(
        app.test_vocal_lines(def, 30),
        vec!["hello world", "second line", "third line"]
    );
}

#[test]
fn set_line_replaces_one_line() {
    let mut app = app_with_project();
    let (def, track) = vocal_section(&mut app, 31);
    let _ = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: track,
            section_id: None,
            text: "line one\nline two".to_owned(),
        },
    );

    let response = call(
        &mut app,
        "vocal.set_line",
        &proto::SetLineParams {
            track_id: track,
            section_id: None,
            line_index: 1,
            text: "replaced two".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_line succeeds");
    assert_eq!(app.test_vocal_lines(def, 31), vec!["line one", "replaced two"]);

    // Out of range.
    let response = call(
        &mut app,
        "vocal.set_line",
        &proto::SetLineParams {
            track_id: track,
            section_id: None,
            line_index: 9,
            text: "nope".to_owned(),
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("out of range"), "unexpected: {message}");
}

#[test]
fn lyrics_on_a_non_vocal_track_error() {
    let mut app = app_with_project();
    // A track with no vocal lane.
    app.test_add_track(32, TrackType::Instrument);
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(32),
            section_id: None,
            text: "x".to_owned(),
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("no vocal lane"), "unexpected: {message}");

    // A wholly unknown track.
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(999),
            section_id: None,
            text: "x".to_owned(),
        },
    );
    expect_error(response, ErrorKind::NotFound);
}

// ---------------- vocal.set_pronunciation / clear ----------------

#[test]
fn set_and_clear_pronunciation() {
    let mut app = app_with_project();

    let response = call(
        &mut app,
        "vocal.set_pronunciation",
        &proto::SetPronunciationParams {
            word: "Lilia".to_owned(),
            phonemes: vec!["l", "ih", "l", "iy", "ah"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
    );
    let _: MutationAck = response.result().expect("set_pronunciation succeeds");

    let dict = app.test_pronunciation_dictionary();
    assert_eq!(dict.len(), 1);
    assert_eq!(dict[0].0, "lilia"); // cleaned + lowercased key
    assert_eq!(dict[0].1, vec!["l", "ih", "l", "iy", "ah"]);

    // Overwriting the same word replaces it (no duplicate).
    let response = call(
        &mut app,
        "vocal.set_pronunciation",
        &proto::SetPronunciationParams {
            word: "lilia".to_owned(),
            phonemes: vec!["l".to_owned(), "iy".to_owned(), "ah".to_owned()],
        },
    );
    let _: MutationAck = response.result().expect("overwrite succeeds");
    let dict = app.test_pronunciation_dictionary();
    assert_eq!(dict.len(), 1);
    assert_eq!(dict[0].1, vec!["l", "iy", "ah"]);

    // Clear it.
    let response = call(
        &mut app,
        "vocal.clear_pronunciation",
        &proto::ClearPronunciationParams { word: "LILIA".to_owned() },
    );
    let _: MutationAck = response.result().expect("clear succeeds");
    assert!(app.test_pronunciation_dictionary().is_empty());

    // Clearing an absent word is not_found.
    let response = call(
        &mut app,
        "vocal.clear_pronunciation",
        &proto::ClearPronunciationParams { word: "lilia".to_owned() },
    );
    expect_error(response, ErrorKind::NotFound);
}

#[test]
fn set_pronunciation_validates_phonemes() {
    let mut app = app_with_project();

    // Empty phonemes rejected.
    let response = call(
        &mut app,
        "vocal.set_pronunciation",
        &proto::SetPronunciationParams {
            word: "word".to_owned(),
            phonemes: vec![],
        },
    );
    expect_error(response, ErrorKind::InvalidParams);

    // A bogus ARPAbet symbol is rejected and named.
    let response = call(
        &mut app,
        "vocal.set_pronunciation",
        &proto::SetPronunciationParams {
            word: "word".to_owned(),
            phonemes: vec!["w".to_owned(), "zzz".to_owned()],
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("zzz"), "names the bad phoneme: {message}");
    assert!(app.test_pronunciation_dictionary().is_empty());
}

// ---------------- vocal.render (job) ----------------

fn job_status(app: &mut Resonance, job_id: resonance_control::ids::JobId) -> JobStatus {
    call(app, "job.status", &StatusParams { job_id })
        .result()
        .expect("job.status succeeds")
}

/// `vocal.render` sings the lane's existing notes and no longer
/// generates them (doc #271 V1), so a lane must be generated before it
/// can be rendered — the flow `vocal.generate` was added for.
fn generate_lane(app: &mut Resonance, track: ProtoTrackId) {
    call(
        app,
        "vocal.generate",
        &proto::GenerateParams {
            track_id: track,
            section_id: None,
            seed: Some(1),
            lyrics: false,
        },
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate succeeds");
}

#[test]
fn render_returns_a_job_and_sets_the_default_voicebank() {
    let mut app = app_with_project();
    let (def, track) = vocal_section(&mut app, 33);
    generate_lane(&mut app, track);
    // The lane starts on the code default (TIGER); render with no
    // voicebank must switch it to the app default (Lilia, doc #265).
    assert_eq!(
        app.test_vocal_voicebank(def, 33),
        Some(resonance_music_theory::VocalVoicebank::Tiger)
    );

    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(track),
            section_id: None,
            voicebank: None,
        },
    );
    let started: JobStarted = response.result().expect("render returns a job");

    // The lane's voicebank is now Lilia.
    assert_eq!(
        app.test_vocal_voicebank(def, 33),
        Some(resonance_music_theory::VocalVoicebank::Lilia)
    );

    // The job exists and is live (the SVS model dir is absent under test,
    // so it never completes — but it must be tracked, not errored).
    let status = job_status(&mut app, started.job_id);
    assert!(
        matches!(status.state, JobState::Pending | JobState::Running),
        "render job should be live, got {:?}",
        status.state
    );
}

#[test]
fn render_accepts_an_explicit_voicebank() {
    let mut app = app_with_project();
    let (def, track) = vocal_section(&mut app, 34);
    generate_lane(&mut app, track);

    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(track),
            section_id: None,
            voicebank: Some("Meiji".to_owned()),
        },
    );
    let _: JobStarted = response.result().expect("render returns a job");
    assert_eq!(
        app.test_vocal_voicebank(def, 34),
        Some(resonance_music_theory::VocalVoicebank::Meiji)
    );

    // Unknown voicebank rejected before any job starts.
    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(track),
            section_id: None,
            voicebank: Some("nightingale".to_owned()),
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("unknown voicebank"), "unexpected: {message}");
}

#[test]
fn render_without_a_vocal_lane_errors() {
    let mut app = app_with_project();
    // No vocal lane anywhere → the no-track path.
    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: None,
            section_id: None,
            voicebank: None,
        },
    );
    expect_error(response, ErrorKind::NotFound);

    // A specific non-vocal track.
    app.test_add_track(35, TrackType::Instrument);
    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(ProtoTrackId(35)),
            section_id: None,
            voicebank: None,
        },
    );
    expect_error(response, ErrorKind::NotFound);
}

#[test]
fn render_with_empty_draft_fails_the_job() {
    let mut app = app_with_project();
    let (def, track) = vocal_section(&mut app, 36);
    // Clear the seeded draft so the render has nothing to sing.
    let _ = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: track,
            section_id: None,
            text: String::new(),
        },
    );
    assert!(app.test_vocal_lines(def, 36).is_empty());

    let response = call(
        &mut app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(track),
            section_id: None,
            voicebank: None,
        },
    );
    // An empty draft is a synchronous validation failure surfaced as an
    // error reply (and the job is failed, not left dangling).
    expect_error(response, ErrorKind::InvalidParams);
}

// ---------------- gating ----------------

#[test]
fn vocal_without_project_is_busy() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            track_id: ProtoTrackId(1),
            section_id: None,
            text: "x".to_owned(),
        },
    );
    expect_error(response, ErrorKind::Busy);
}
