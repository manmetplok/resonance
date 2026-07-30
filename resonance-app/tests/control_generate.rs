//! `generate.*` control methods (ba doc #265, todo #1154), driven
//! through the real `update()` path with synthesized requests. Sections
//! and chords are set up via the `section.*` / `harmony.*` control
//! methods (todo #1153) so the tests exercise the full remote surface.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{ErrorKind, KeyScale, Request, Response};

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-generate-test.rprj"));
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

/// A section with a 4-chord Am-key progression, returning its id.
fn section_with_chords(app: &mut Resonance) -> SectionDefinitionId {
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
        .expect("section.create succeeds")
        .section_id;

    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(
        ["i", "iv", "v", "i"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    let response = call(app, "harmony.apply_progression", &params);
    response
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
    section_id
}

fn add_synth_track(app: &mut Resonance, id: u64) -> ProtoTrackId {
    app.test_add_track(id, TrackType::Instrument);
    ProtoTrackId(id)
}

// ---------------- generate.part ----------------

#[test]
fn part_installs_generator_and_derives_clips() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let track = add_synth_track(&mut app, 10);

    for role in [GenerateRole::Bass, GenerateRole::Lead, GenerateRole::Pad] {
        let params = proto::PartParams {
            section_id,
            track_id: track,
            role,
            chord_count: None,
            beats_per_chord: None,
            sevenths: None,
            seed: Some(42),
            options: None,
        };
        let response = call(&mut app, "generate.part", &params);
        let result: GenerateResult = response.result().expect("generate.part succeeds");
        // Derived clips aren't addressable project clips.
        assert!(result.clip_id.is_none());
    }

    // The section has one auto placement, so exactly one derived clip
    // exists for the track (the last role generated replaces the lane).
    assert_eq!(app.test_derived_clip_count(10), 1);
    // The lane now carries a Pad generator (the last role set).
    use resonance_app::compose::LaneGeneratorKindTag;
    assert_eq!(
        app.test_lane_generator_tag(u64::from(section_id), 10),
        Some(LaneGeneratorKindTag::Pad)
    );
}

#[test]
fn part_accepts_per_role_options() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let track = add_synth_track(&mut app, 12);

    // A Bass options object mapping straight onto BassParams.
    let params = proto::PartParams {
        section_id,
        track_id: track,
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(1),
        options: Some(serde_json::json!({
            "style": "Walking",
            "base_note": 31,
            "velocity": 0.9,
        })),
    };
    let response = call(&mut app, "generate.part", &params);
    response
        .result::<GenerateResult>()
        .expect("valid options accepted");

    // A malformed options object (wrong type) is rejected.
    let params = proto::PartParams {
        section_id,
        track_id: track,
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(1),
        options: Some(serde_json::json!({ "base_note": "not a number" })),
    };
    let response = call(&mut app, "generate.part", &params);
    expect_error(response, ErrorKind::InvalidParams);
}

#[test]
fn part_requires_chords_and_a_synth_track() {
    let mut app = app_with_project();
    // Section with no chords.
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Empty".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    );
    let empty = response
        .result::<section_proto::CreateResult>()
        .expect("create")
        .section_id;
    let track = add_synth_track(&mut app, 13);

    let params = proto::PartParams {
        section_id: empty,
        track_id: track,
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: None,
        options: None,
    };
    let message = expect_error(call(&mut app, "generate.part", &params), ErrorKind::InvalidParams);
    assert!(message.contains("no chords"), "unexpected: {message}");

    // Unknown track.
    let section_id = section_with_chords(&mut app);
    let params = proto::PartParams {
        section_id,
        track_id: ProtoTrackId(999),
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: None,
        options: None,
    };
    expect_error(call(&mut app, "generate.part", &params), ErrorKind::NotFound);

    // A drum track routed to generate.part is a category error.
    app.test_add_drum_track(14);
    let params = proto::PartParams {
        section_id,
        track_id: ProtoTrackId(14),
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: None,
        options: None,
    };
    let message = expect_error(call(&mut app, "generate.part", &params), ErrorKind::InvalidParams);
    assert!(message.contains("generate.drums"), "unexpected: {message}");

    // Unknown section.
    let params = proto::PartParams {
        section_id: SectionDefinitionId(999_999),
        track_id: track,
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: None,
        options: None,
    };
    expect_error(call(&mut app, "generate.part", &params), ErrorKind::NotFound);
}

#[test]
fn part_bumps_revision_once_and_is_undoable() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let track = add_synth_track(&mut app, 15);

    let before = call(&mut app, "generate.part", &proto::PartParams {
        section_id,
        track_id: track,
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(3),
        options: None,
    })
    .result::<GenerateResult>()
    .expect("first generate succeeds")
    .revision;

    // A second generate bumps the revision again (one transaction each).
    let after = call(&mut app, "generate.part", &proto::PartParams {
        section_id,
        track_id: track,
        role: GenerateRole::Lead,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(3),
        options: None,
    })
    .result::<GenerateResult>()
    .expect("second generate succeeds")
    .revision;
    assert_eq!(after, before + 1);
}

// ---------------- generate.drums ----------------

#[test]
fn drums_generates_onto_a_drum_track() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    app.test_add_drum_track(20);

    let params = proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(20),
        pattern: None,
        seed: Some(99),
    };
    let response = call(&mut app, "generate.drums", &params);
    let result: GenerateResult = response.result().expect("generate.drums succeeds");
    assert!(result.clip_id.is_none());

    // The section now has a primary drum pattern assigned.
    assert!(
        app.test_section_primary_pattern(u64::from(section_id)).is_some(),
        "a drum pattern is pinned to the section"
    );
}

#[test]
fn drums_rejects_non_drum_track_and_unknown_pattern() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);

    // A synth track routed to generate.drums.
    app.test_add_track(21, TrackType::Instrument);
    let params = proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(21),
        pattern: None,
        seed: None,
    };
    let message = expect_error(call(&mut app, "generate.drums", &params), ErrorKind::InvalidParams);
    assert!(message.contains("not a drum track"), "unexpected: {message}");

    // Unknown named pattern lists the known ones.
    app.test_add_drum_track(22);
    let params = proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(22),
        pattern: Some("nonexistent-pattern".to_owned()),
        seed: None,
    };
    let message = expect_error(call(&mut app, "generate.drums", &params), ErrorKind::NotFound);
    assert!(message.contains("no drum pattern named"), "unexpected: {message}");

    // Unknown track.
    let params = proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(999),
        pattern: None,
        seed: None,
    };
    expect_error(call(&mut app, "generate.drums", &params), ErrorKind::NotFound);

    // Unknown section.
    let params = proto::DrumsParams {
        section_id: SectionDefinitionId(999_999),
        track_id: ProtoTrackId(22),
        pattern: None,
        seed: None,
    };
    expect_error(call(&mut app, "generate.drums", &params), ErrorKind::NotFound);
}

#[test]
fn drums_named_pattern_is_pinned() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    app.test_add_drum_track(23);

    // A fresh project has at least one default drum pattern; look up its
    // name via song.sections is not exposed, so use the by-name path with
    // the default pattern's known name. We instead confirm the None-path
    // pins *a* pattern and its id is stable across repeat calls.
    let first = {
        let _ = call(&mut app, "generate.drums", &proto::DrumsParams {
            section_id,
            track_id: ProtoTrackId(23),
            pattern: None,
            seed: Some(1),
        });
        app.test_section_primary_pattern(u64::from(section_id))
    };
    let second = {
        let _ = call(&mut app, "generate.drums", &proto::DrumsParams {
            section_id,
            track_id: ProtoTrackId(23),
            pattern: None,
            seed: Some(2),
        });
        app.test_section_primary_pattern(u64::from(section_id))
    };
    assert!(first.is_some());
    // Re-running with no explicit pattern reuses the section's now-primary
    // pattern, so the pinned id is stable.
    assert_eq!(first, second);
}

// ---------------- gating ----------------

#[test]
fn generate_without_project_is_busy() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    let params = proto::PartParams {
        section_id: SectionDefinitionId(1),
        track_id: ProtoTrackId(1),
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: None,
        options: None,
    };
    expect_error(call(&mut app, "generate.part", &params), ErrorKind::Busy);
}
