//! `song.vocal` per-lane identity + counts, and `section_id` addressing
//! on the `vocal.*` mutations (ba doc #269 FR-3/FR-7).
//!
//! Before this, `song.vocal` reported `{lines, render_state, revision,
//! track_id}` with no section identity, so a client could not tell which
//! lane a write had hit — `vocal.set_lyrics` silently resolves a track's
//! first vocal lane by start bar. These tests set up a track singing in
//! two sections and pin down both halves of the fix.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::VocalView;
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, MutationAck, Request, Response};
use crate::common::roundtrip;

const TRACK: u64 = 50;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-vocal-lanes.rprj"));
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

/// Create a section placed at `start_bar` (1-based) with a vocal lane on
/// `TRACK`, returning its definition id.
fn vocal_lane(app: &mut Resonance, name: &str, start_bar: u32) -> u64 {
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
    app.test_install_vocal_lane(def, TRACK);
    def
}

/// A track singing in two sections: Verse at bar 1, Chorus at bar 5.
fn two_lane_app() -> (Resonance, u64, u64) {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);
    let verse = vocal_lane(&mut app, "Verse", 1);
    let chorus = vocal_lane(&mut app, "Chorus", 5);
    (app, verse, chorus)
}

fn vocal_view(app: &mut Resonance) -> VocalView {
    roundtrip(
        app,
        Request::new(9, "song.vocal", &serde_json::json!({ "track_id": TRACK })).unwrap(),
    )
    .result()
    .expect("song.vocal succeeds")
}

/// Give a section a chord grid, so `vocal.generate` can derive a melody
/// for its lane.
fn add_chords(app: &mut Resonance, def: u64) {
    let mut params = resonance_control::methods::harmony::ApplyProgressionParams::for_section(
        SectionDefinitionId(def),
    );
    params.key = Some(resonance_control::KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(
        ["i", "iv", "v", "i"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    call(app, "harmony.apply_progression", &params)
        .result::<resonance_control::methods::harmony::ApplyProgressionResult>()
        .expect("progression applies");
}

fn render(app: &mut Resonance, section_id: Option<u64>) -> Response {
    call(
        app,
        "vocal.render",
        &proto::RenderParams {
            track_id: Some(ProtoTrackId(TRACK)),
            section_id: section_id.map(SectionDefinitionId),
            voicebank: None,
        },
    )
}

/// `vocal.render` used to hardcode the track's first vocal lane, so a
/// track singing in several sections could only ever re-render its
/// first — every other lane stayed frozen at its first render (doc #271
/// V2). An explicit `section_id` must reach the lane it names.
#[test]
fn render_targets_the_lane_named_by_section_id() {
    let (mut app, verse, chorus) = two_lane_app();
    add_chords(&mut app, chorus);

    // Only the chorus lane has notes.
    call(
        &mut app,
        "vocal.generate",
        &proto::GenerateParams {
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(chorus)),
            seed: Some(3),
            lyrics: false,
        },
    )
    .result::<proto::GenerateResult>()
    .expect("vocal.generate on the chorus lane succeeds");

    // Naming the chorus renders it.
    render(&mut app, Some(chorus))
        .result::<resonance_control::job::JobStarted>()
        .expect("the named lane renders");

    // Omitting section_id covers the whole track, not its first lane:
    // the verse has no notes and is skipped, and the render still runs
    // for the chorus. This used to resolve the first lane alone and fail
    // with "no notes" — the same resolution that silently left lanes
    // 2..n stale on a track where every lane *could* render.
    render(&mut app, None)
        .result::<resonance_control::job::JobStarted>()
        .expect("a track-level render skips the empty lane and renders the rest");

    // A section the track does not sing in is a precise error, not a
    // silent fallback to the first lane.
    let orphan = {
        let response = call(
            &mut app,
            "section.create",
            &section_proto::CreateParams {
                name: "Instrumental".to_owned(),
                length_bars: 4,
                scale: None,
                place: false,
            },
        );
        u64::from(
            response
                .result::<section_proto::CreateResult>()
                .expect("section.create succeeds")
                .section_id,
        )
    };
    let message = expect_error(render(&mut app, Some(orphan)), ErrorKind::InvalidParams);
    assert!(message.contains("no vocal lane"), "unexpected: {message}");
    let _ = verse;
}

// ---------------- FR-3/FR-7: song.vocal lane identity ----------------

#[test]
fn lanes_report_section_identity_in_placement_order() {
    let (mut app, verse, chorus) = two_lane_app();

    let view = vocal_view(&mut app);
    let ids: Vec<u64> = view
        .lanes
        .iter()
        .map(|l| u64::from(l.definition_id))
        .collect();
    assert_eq!(ids, vec![verse, chorus], "lanes read in placement order");
    assert_eq!(view.lanes[0].name, "Verse");
    assert_eq!(view.lanes[1].name, "Chorus");
    // Placement bars are 1-based on the wire.
    assert_eq!(view.lanes[0].start_bar, Some(1));
    assert_eq!(view.lanes[1].start_bar, Some(5));
}

#[test]
fn an_unplaced_lane_reports_no_start_bar() {
    let mut app = app_with_project();
    app.test_add_track(TRACK, TrackType::Vocal);
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Outro".to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    );
    let def = u64::from(
        response
            .result::<section_proto::CreateResult>()
            .expect("create succeeds")
            .section_id,
    );
    app.test_install_vocal_lane(def, TRACK);

    let view = vocal_view(&mut app);
    assert_eq!(view.lanes.len(), 1);
    assert_eq!(view.lanes[0].start_bar, None);
}

#[test]
fn counts_are_reported_per_lane_and_flag_lyrics_without_notes() {
    let (mut app, verse, _chorus) = two_lane_app();
    // Two lines of one syllable each, so the count is the engine's, not
    // a guess: the whole point of reporting it is that G2P decides
    // syllabification, not a by-eye reading.
    let _ = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: true,
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(verse)),
            text: "go\nstay".to_owned(),
        },
    );

    let view = vocal_view(&mut app);
    let lane = &view.lanes[0];
    assert_eq!(u64::from(lane.definition_id), verse);
    assert_eq!(lane.syllable_count, 2);
    // Lyrics with nothing generated to sing them IS the mismatch worth
    // flagging: a render here produces nothing the client asked for.
    // This used to report `false` (the flag required note_count > 0),
    // which gave false confidence right before a render — doc #271.
    // "Not generated yet" stays legible as note_count == 0.
    assert_eq!(lane.note_count, 0);
    assert!(lane.counts_mismatch);
    // The counts are per lane: the Chorus lane still carries its own
    // (default) draft and reports its own, different, count.
    assert_ne!(view.lanes[1].syllable_count, lane.syllable_count);
}

/// A lane whose clip exists but is not in the derived-clip map must
/// still report its notes.
///
/// `lane_note_count` consulted only that map, so a lane reported
/// `note_count: 0` while `song.notes` on its clip returned notes — which
/// reads as "not generated" and silenced the mismatch flag (doc #271).
#[test]
fn note_count_finds_the_lane_clip_without_the_derived_map() {
    let (mut app, verse, _chorus) = two_lane_app();

    // A MIDI clip on the lane's track at the section's placement bar,
    // mirrored the way the engine echo does — but with the derived-clip
    // map left empty, as a stale or half-rebuilt map would be.
    let start = app.test_tempo_map().bar_to_sample(0);
    app.test_apply_engine_event(resonance_audio::types::AudioEvent::MidiClipCreated {
        clip_id: 900_001,
        track_id: TRACK,
        start_sample: start,
        duration_ticks: 4 * 4 * resonance_audio::types::TICKS_PER_QUARTER_NOTE,
        name: "Verse · Vox".to_owned(),
        notes: vec![resonance_audio::types::MidiNote {
            note: 62,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });

    let view = vocal_view(&mut app);
    let lane = view
        .lanes
        .iter()
        .find(|l| u64::from(l.definition_id) == verse)
        .expect("the verse lane is listed");
    assert_eq!(lane.note_count, 1, "the lane's notes are counted");
}

// ---------------- Intelligibility pre-flight ----------------

/// `song.vocal` must let a client tell — without rendering and listening
/// — that a note is too short to articulate what it has been given.
///
/// This is the case that produced "vocals are not understandable at all":
/// a phoneme-dense syllable on a short note renders successfully and
/// comes out as a smear, with nothing in the API saying so.
#[test]
fn song_vocal_flags_notes_too_short_to_articulate() {
    let (mut app, verse, _chorus) = two_lane_app();
    // One word, deliberately stored unbroken, so all nine of its
    // phonemes land on a single note.
    let _ = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: false,
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(verse)),
            text: "resolution".to_owned(),
        },
    );
    // A sixteenth-note clip on the lane's track at its placement bar.
    let start = app.test_tempo_map().bar_to_sample(0);
    app.test_apply_engine_event(resonance_audio::types::AudioEvent::MidiClipCreated {
        clip_id: 900_002,
        track_id: TRACK,
        start_sample: start,
        duration_ticks: 4 * 4 * resonance_audio::types::TICKS_PER_QUARTER_NOTE,
        name: "Verse · Vox".to_owned(),
        notes: vec![resonance_audio::types::MidiNote {
            note: 64,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: resonance_audio::types::TICKS_PER_QUARTER_NOTE / 4,
        }],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });

    let view = vocal_view(&mut app);
    let lane = view
        .lanes
        .iter()
        .find(|l| u64::from(l.definition_id) == verse)
        .expect("the verse lane is listed");

    assert_eq!(lane.notes.len(), 1, "per-note report present");
    let note = &lane.notes[0];
    assert_eq!(note.phoneme_count, 9, "{:?}", note.phonemes);
    assert!(
        note.too_short,
        "9 phonemes in {} ms (needs {} ms) should be flagged",
        note.duration_ms, note.min_duration_ms
    );
    assert!(note.min_duration_ms > note.duration_ms);
    assert_eq!(lane.short_note_count, 1);

    // The voicebank's comfortable range travels with the lane so a
    // client can also see whether a melody sits where it sings clearly.
    let range = lane.comfortable_range.as_ref().expect("range reported");
    assert!(range.low < range.high);
    assert!(!range.low_name.is_empty() && !range.high_name.is_empty());
    assert!(lane.voicebank.is_some());
    assert!(!note.out_of_range, "E4 is comfortable for every bank");
    assert_eq!(lane.out_of_range_note_count, 0);
}

// ---------------- FR-3 part 2: section_id addressing ----------------

#[test]
fn section_id_writes_the_addressed_lane() {
    let (mut app, verse, chorus) = two_lane_app();

    let verse_before = app.test_vocal_lines(verse, TRACK);

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: true,
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(chorus)),
            text: "sing it louder".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_lyrics succeeds");

    // `louder` is two syllables, so the stored line carries a break.
    assert_eq!(
        app.test_vocal_lines(chorus, TRACK),
        vec!["sing it lou\u{00B7}der"]
    );
    assert_eq!(
        app.test_vocal_lines(verse, TRACK),
        verse_before,
        "the first lane is untouched — before FR-3 this write landed there"
    );
}

#[test]
fn an_omitted_section_id_still_writes_the_first_lane() {
    let (mut app, verse, chorus) = two_lane_app();

    let chorus_before = app.test_vocal_lines(chorus, TRACK);

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: true,
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            text: "first lane".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_lyrics succeeds");

    assert_eq!(app.test_vocal_lines(verse, TRACK), vec!["first lane"]);
    assert_eq!(app.test_vocal_lines(chorus, TRACK), chorus_before);
}

#[test]
fn set_line_addresses_the_same_lane() {
    let (mut app, verse, chorus) = two_lane_app();
    let verse_before = app.test_vocal_lines(verse, TRACK);
    let _ = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: true,
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(chorus)),
            text: "one\ntwo".to_owned(),
        },
    );

    let response = call(
        &mut app,
        "vocal.set_line",
        &proto::SetLineParams {
            syllabify: true,
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(chorus)),
            line_index: 1,
            text: "TWO".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("set_line succeeds");
    assert_eq!(app.test_vocal_lines(chorus, TRACK), vec!["one", "TWO"]);
    assert_eq!(app.test_vocal_lines(verse, TRACK), verse_before);
}

#[test]
fn a_section_without_a_vocal_lane_on_the_track_is_rejected() {
    let (mut app, _verse, _chorus) = two_lane_app();
    // A section that exists but carries no vocal lane for this track.
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Bridge".to_owned(),
            length_bars: 4,
            scale: None,
            place: false,
        },
    );
    let bare = u64::from(
        response
            .result::<section_proto::CreateResult>()
            .expect("create succeeds")
            .section_id,
    );

    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: true,
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(bare)),
            text: "nope".to_owned(),
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("no vocal lane"), "{message}");
}

#[test]
fn an_unknown_section_id_is_not_found() {
    let (mut app, _verse, _chorus) = two_lane_app();
    let response = call(
        &mut app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: true,
            track_id: ProtoTrackId(TRACK),
            section_id: Some(SectionDefinitionId(9999)),
            text: "nope".to_owned(),
        },
    );
    let message = expect_error(response, ErrorKind::NotFound);
    assert!(message.contains("section definition"), "{message}");
}
