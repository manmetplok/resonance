//! Melisma holds end to end (vocal-mcp-control-and-quality.md §6, §8):
//! the vocal roll's `s` key writes `_` into the lane's lyrics, the
//! generator's own `SyllableMode` holds ride in the clip's per-note side
//! table, and `song.vocal` tells them apart (`slur: user | auto`) along
//! with each note's `g2p` source and `syllable_warning`.

use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::{VocalG2pSource, VocalLaneView, VocalSlurKind, VocalView};
use resonance_control::methods::vocal as proto;
use resonance_control::{KeyScale, MutationAck, Request, Response};
use serde_json::json;

use crate::common::roundtrip;

const TRACK: u64 = 61;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_install_shipped_voicebanks();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-vocal-holds.rprj"));
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// A placed 4-bar section with chords and a vocal lane on `TRACK`.
fn vocal_lane(app: &mut Resonance) -> u64 {
    let section_id = call(
        app,
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
    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    let _ = call(app, "harmony.apply_progression", &params);
    app.test_add_track(TRACK, TrackType::Vocal);
    call(
        app,
        "section.set_lane_generator",
        &section_proto::SetLaneGeneratorParams {
            section_id,
            track_id: ProtoTrackId(TRACK),
            kind: section_proto::LaneKind::Vocal,
            seed: None,
            options: None,
        },
    )
    .result::<section_proto::SetLaneGeneratorResult>()
    .expect("the vocal lane installs");
    u64::from(section_id)
}

fn set_lyrics(app: &mut Resonance, text: &str, syllabify: bool) {
    let _: MutationAck = call(
        app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify,
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            text: text.to_owned(),
        },
    )
    .result()
    .expect("set_lyrics succeeds");
}

/// Melody only, so the lyrics (and their `_` holds) stay.
fn generate(app: &mut Resonance, seed: u64) -> u64 {
    let result: proto::GenerateResult = call(
        app,
        "vocal.generate",
        &proto::GenerateParams {
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            seed: Some(seed),
            lyrics: false,
        },
    )
    .result()
    .expect("vocal.generate succeeds");
    u64::from(result.clip_id)
}

fn lane(app: &mut Resonance) -> VocalLaneView {
    let view: VocalView = call(app, "song.vocal", &json!({ "track_id": TRACK }))
        .result()
        .expect("song.vocal succeeds");
    view.lanes.into_iter().next().expect("one vocal lane")
}

fn press_s(app: &mut Resonance, clip_id: u64, note_index: usize) {
    let _ = app.update(Message::MidiEditor(MidiEditorMessage::ToggleSlur {
        clip_id,
        note_index,
    }));
}

fn slurs(lane: &VocalLaneView) -> Vec<Option<VocalSlurKind>> {
    lane.notes.iter().map(|n| n.slur).collect()
}

fn undo_depth(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

#[test]
fn s_writes_a_hold_into_the_lyrics_that_survives_generate() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    set_lyrics(&mut app, "love me tender", true);
    assert_eq!(app.test_vocal_lines(def, TRACK), ["love me ten\u{00B7}der"]);
    let clip = generate(&mut app, 5);
    assert_eq!(lane(&mut app).note_count, 4);

    // `s` on "me" makes it hold "love": one undo entry, a `_` in the
    // lyrics, no per-note marker.
    let depth = undo_depth(&app);
    press_s(&mut app, clip, 1);
    assert_eq!(undo_depth(&app), depth + 1, "one undo entry per keypress");
    assert_eq!(app.test_vocal_lines(def, TRACK), ["love _ me ten\u{00B7}der"]);
    let view = lane(&mut app);
    assert_eq!(view.notes[1].slur, Some(VocalSlurKind::User));
    assert_eq!(view.syllable_count, 5);
    assert!(
        app.compose_state().vocal_audio.clip_lyrics[&clip].iter().all(String::is_empty),
        "a user hold lives in the lyrics, not the side table"
    );

    // A regenerate keeps it: the lyrics carry the hold.
    let clip = generate(&mut app, 6);
    let view = lane(&mut app);
    assert_eq!(view.note_count, 5);
    assert!(!view.counts_mismatch);
    assert_eq!(slurs(&view), [None, Some(VocalSlurKind::User), None, None, None]);
    assert_eq!(app.test_vocal_lines(def, TRACK), ["love _ me ten\u{00B7}der"]);

    // `s` on the user hold takes the `_` out again; undo puts it back.
    press_s(&mut app, clip, 1);
    assert_eq!(app.test_vocal_lines(def, TRACK), ["love me ten\u{00B7}der"]);
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_vocal_lines(def, TRACK), ["love _ me ten\u{00B7}der"]);
}

#[test]
fn s_inside_a_word_writes_an_in_word_hold() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    set_lyrics(&mut app, "ten\u{00B7}der love", false);
    let clip = generate(&mut app, 2);
    press_s(&mut app, clip, 1);
    assert_eq!(app.test_vocal_lines(def, TRACK), ["ten\u{00B7}_\u{00B7}der love"]);
    let view = lane(&mut app);
    assert_eq!(view.notes[1].slur, Some(VocalSlurKind::User));
}

#[test]
fn s_on_a_lines_first_note_does_nothing() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    set_lyrics(&mut app, "hold on\nlet go", true);
    let clip = generate(&mut app, 3);
    let before = app.test_vocal_lines(def, TRACK);
    press_s(&mut app, clip, 0);
    press_s(&mut app, clip, 2);
    assert_eq!(app.test_vocal_lines(def, TRACK), before);
    assert!(slurs(&lane(&mut app)).iter().all(Option::is_none));
}

/// The first melismatic seed (of a few) whose melody has generator holds.
fn melismatic_lane(app: &mut Resonance) -> u64 {
    let _: proto::ParamsResult = call(
        app,
        "vocal.set_params",
        &json!({ "track_id": TRACK, "params": { "syllable_mode": "melismatic" } }),
    )
    .result()
    .expect("set_params succeeds");
    set_lyrics(app, "you are the sun\nshining on me\nhold me now\nnever let go", true);
    for seed in 1..20 {
        let clip = generate(app, seed);
        if lane(app).notes.iter().any(|n| n.slur == Some(VocalSlurKind::Auto)) {
            return clip;
        }
    }
    panic!("a melismatic lane makes generator holds");
}

#[test]
fn generator_holds_read_auto_and_do_not_count_as_lyrics() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    let clip = melismatic_lane(&mut app);
    let view = lane(&mut app);
    let autos = view.notes.iter().filter(|n| n.slur == Some(VocalSlurKind::Auto)).count();
    assert!(autos > 0);
    assert_eq!(view.note_count, view.syllable_count + autos, "auto holds are extra notes");
    assert!(!view.counts_mismatch, "auto holds are not lyric units");
    // They live in the per-note side table, never in the lyrics.
    assert!(app.test_vocal_lines(def, TRACK).iter().all(|l| !l.contains('_')));
    let marks = &app.compose_state().vocal_audio.clip_lyrics[&clip];
    assert_eq!(marks.iter().filter(|m| *m == "+").count(), autos);
    // Every note carries a pronunciation source; holds inherit it.
    assert!(view.notes.iter().all(|n| n.g2p == Some(VocalG2pSource::Cmu)));
}

#[test]
fn s_on_a_generator_hold_converts_it_to_a_user_hold() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    let clip = melismatic_lane(&mut app);
    let view = lane(&mut app);
    let i = view
        .notes
        .iter()
        .position(|n| n.slur == Some(VocalSlurKind::Auto))
        .expect("an auto hold");
    press_s(&mut app, clip, i);
    let after = lane(&mut app);
    assert_eq!(after.notes[i].slur, Some(VocalSlurKind::User));
    assert!(app.test_vocal_lines(def, TRACK).iter().any(|l| l.contains('_')));
    assert!(app.compose_state().vocal_audio.clip_lyrics[&clip][i].is_empty());
    // Same notes, one hold moved from the side table into the lyrics:
    // still no mismatch, and nothing else changed.
    assert!(!after.counts_mismatch);
    assert_eq!(after.note_count, view.note_count);
    for (j, (a, b)) in view.notes.iter().zip(&after.notes).enumerate() {
        if j != i && b.slur != Some(VocalSlurKind::User) {
            assert_eq!(a.slur, b.slur, "note {j}");
        }
        assert_eq!(a.syllable, b.syllable, "note {j} sings the same");
    }
}

#[test]
fn notes_report_their_pronunciation_source_and_split_warning() {
    let mut app = app_with_project();
    let _ = vocal_lane(&mut app);
    // `lo·ve` breaks a one-vowel word in two; `zorbl` is in no dictionary.
    set_lyrics(&mut app, "lo\u{00B7}ve zorbl sun", false);
    let _ = generate(&mut app, 4);
    let _: MutationAck = call(
        &mut app,
        "vocal.set_pronunciation",
        &json!({ "word": "sun", "phonemes": ["s", "ah", "n"] }),
    )
    .result()
    .expect("set_pronunciation succeeds");
    let view = lane(&mut app);
    let g2p: Vec<Option<VocalG2pSource>> = view.notes.iter().map(|n| n.g2p).collect();
    assert_eq!(
        g2p,
        [
            Some(VocalG2pSource::Cmu),
            Some(VocalG2pSource::Cmu),
            Some(VocalG2pSource::Rules),
            Some(VocalG2pSource::ProjectDict),
        ]
    );
    let warned: Vec<bool> = view.notes.iter().map(|n| n.syllable_warning).collect();
    assert_eq!(warned, [true, true, false, false]);
}
