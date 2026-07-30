//! `section.*` / `harmony.*` control methods (ba doc #265, todo #1153),
//! driven through the real `update()` path with synthesized requests —
//! no live socket needed. State is asserted through the `song.sections`
//! view, i.e. the same wire surface a remote client sees.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::SectionsView;
use resonance_control::{ErrorKind, KeyScale, MutationAck, Request, Response};

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-section-harmony-test.rprj"));
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
    roundtrip(
        app,
        Request::new(1, method, params).expect("params serialize"),
    )
}

fn sections_view(app: &mut Resonance) -> SectionsView {
    roundtrip(app, Request::without_params(1, "song.sections"))
        .result()
        .expect("song.sections succeeds")
}

fn key(tonic: &str, scale: &str) -> KeyScale {
    KeyScale {
        tonic: tonic.to_owned(),
        scale: scale.to_owned(),
    }
}

/// Create a section and return its id.
fn create_section(app: &mut Resonance, name: &str, length_bars: u32) -> u64 {
    let response = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: name.to_owned(),
            length_bars,
            scale: None,
            place: true,
        },
    );
    let result: section_proto::CreateResult = response.result().expect("section.create succeeds");
    result.section_id.into()
}

fn chord_symbols(view: &SectionsView, section_id: u64) -> Vec<String> {
    view.definitions
        .iter()
        .find(|d| u64::from(d.id) == section_id)
        .expect("definition present")
        .chords
        .iter()
        .map(|c| c.symbol.clone())
        .collect()
}

fn expect_error(response: Response, kind: ErrorKind) -> String {
    let error = response.error.expect("expected an error reply");
    assert_eq!(error.kind(), kind, "unexpected error kind: {}", error.message);
    error.message
}

// ---------------- section.* ----------------

#[test]
fn create_reports_id_and_auto_places() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let view = sections_view(&mut app);
    let def = view
        .definitions
        .iter()
        .find(|d| u64::from(d.id) == id)
        .expect("created definition visible");
    assert_eq!(def.name, "Verse");
    assert_eq!(def.length_bars, 4);
    assert!(def.scale.is_none());
    assert!(def.chords.is_empty());
    // The app auto-places a new section at the first free bar (bar 1 on
    // the 1-based wire).
    let placement = view
        .placements
        .iter()
        .find(|p| u64::from(p.definition_id) == id)
        .expect("auto placement visible");
    assert_eq!(placement.start_bar, 1);
}

#[test]
fn place_false_creates_the_definition_only() {
    // ba doc #269 FR-6: the implicit placement made create-then-place
    // fail as an overlap, so a client could not build its own
    // arrangement without a remove-placement pass in between.
    let mut app = app_with_project();
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
    let id: u64 = response
        .result::<section_proto::CreateResult>()
        .expect("create succeeds")
        .section_id
        .into();

    let view = sections_view(&mut app);
    assert!(
        view.definitions.iter().any(|d| u64::from(d.id) == id),
        "the definition exists"
    );
    assert!(
        !view
            .placements
            .iter()
            .any(|p| u64::from(p.definition_id) == id),
        "but nothing was placed"
    );

    // And the deliberate placement the implicit one used to block now
    // lands where the client asked for it.
    let response = call(
        &mut app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: id.into(),
            start_bar: 9,
        },
    );
    response
        .result::<section_proto::PlaceResult>()
        .expect("section.place succeeds after place: false");
    let view = sections_view(&mut app);
    let placement = view
        .placements
        .iter()
        .find(|p| u64::from(p.definition_id) == id)
        .expect("placement visible");
    assert_eq!(placement.start_bar, 9);
}

#[test]
fn place_defaults_to_true_when_omitted() {
    // Absent `place` on the wire keeps the historical behaviour, so
    // existing clients are unaffected.
    let mut app = app_with_project();
    let response = roundtrip(
        &mut app,
        Request::new(
            1,
            "section.create",
            &serde_json::json!({ "name": "Verse", "length_bars": 4 }),
        )
        .expect("params serialize"),
    );
    let id: u64 = response
        .result::<section_proto::CreateResult>()
        .expect("create succeeds")
        .section_id
        .into();
    let view = sections_view(&mut app);
    assert!(
        view.placements
            .iter()
            .any(|p| u64::from(p.definition_id) == id),
        "an omitted place still auto-places"
    );
}

#[test]
fn create_with_scale_sets_it() {
    let mut app = app_with_project();
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Chorus".to_owned(),
            length_bars: 8,
            scale: Some(key("A", "minor")),
            place: true,
        },
    );
    let result: section_proto::CreateResult = response.result().expect("create succeeds");

    let view = sections_view(&mut app);
    let def = view
        .definitions
        .iter()
        .find(|d| d.id == result.section_id)
        .expect("definition present");
    let scale = def.scale.as_ref().expect("scale set");
    assert_eq!(scale.tonic, "A");
    assert_eq!(scale.scale, "minor");
}

#[test]
fn create_validates_params() {
    let mut app = app_with_project();
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "  ".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    );
    expect_error(response, ErrorKind::InvalidParams);

    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 0,
            scale: None,
            place: true,
        },
    );
    expect_error(response, ErrorKind::InvalidParams);
}

#[test]
fn mutations_without_project_are_busy() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    let response = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    );
    let message = expect_error(response, ErrorKind::Busy);
    assert!(message.contains("no active project"));
}

#[test]
fn rename_and_resize() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let response = call(
        &mut app,
        "section.rename",
        &section_proto::RenameParams {
            section_id: id.into(),
            name: "Verse A".to_owned(),
        },
    );
    let _: MutationAck = response.result().expect("rename succeeds");

    let response = call(
        &mut app,
        "section.resize",
        &section_proto::ResizeParams {
            section_id: id.into(),
            length_bars: 8,
        },
    );
    let _: MutationAck = response.result().expect("resize succeeds");

    let view = sections_view(&mut app);
    let def = view
        .definitions
        .iter()
        .find(|d| u64::from(d.id) == id)
        .expect("definition present");
    assert_eq!(def.name, "Verse A");
    assert_eq!(def.length_bars, 8);

    let response = call(
        &mut app,
        "section.rename",
        &section_proto::RenameParams {
            section_id: 999_999.into(),
            name: "x".to_owned(),
        },
    );
    expect_error(response, ErrorKind::NotFound);
}

#[test]
fn resize_that_strands_chords_is_rejected() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);
    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id: id.into(),
            start_beat: 12.0,
            duration_beats: 4.0,
            symbol: "C".to_owned(),
        },
    );
    let _: harmony_proto::AddChordResult = response.result().expect("chord added");

    // Shrinking to 2 bars (8 beats) would strand the chord at beats 12..16.
    let response = call(
        &mut app,
        "section.resize",
        &section_proto::ResizeParams {
            section_id: id.into(),
            length_bars: 2,
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("shrink"), "unexpected message: {message}");
}

#[test]
fn delete_requires_confirmation_then_cascades() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let response = call(
        &mut app,
        "section.delete",
        &section_proto::DeleteParams {
            section_id: id.into(),
            confirm: false,
        },
    );
    let message = expect_error(response, ErrorKind::NeedsConfirmation);
    assert!(message.contains("Verse"), "summary names the section: {message}");
    assert!(message.contains("1 arrangement placement"), "summary counts placements: {message}");

    // Still there.
    assert_eq!(sections_view(&mut app).definitions.len(), 1);

    let response = call(
        &mut app,
        "section.delete",
        &section_proto::DeleteParams {
            section_id: id.into(),
            confirm: true,
        },
    );
    let _: MutationAck = response.result().expect("confirmed delete succeeds");

    let view = sections_view(&mut app);
    assert!(view.definitions.is_empty());
    assert!(view.placements.is_empty(), "placements cascade with the definition");
}

#[test]
fn place_and_remove_placement() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    // The wire is 1-based: bar 5 is app bar 4 (right after the auto
    // placement at bars 1-4).
    let response = call(
        &mut app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: id.into(),
            start_bar: 5,
        },
    );
    let placed: section_proto::PlaceResult = response.result().expect("place succeeds");

    let view = sections_view(&mut app);
    assert_eq!(view.placements.len(), 2);
    let second = view
        .placements
        .iter()
        .find(|p| p.id == placed.placement_id)
        .expect("new placement visible");
    assert_eq!(second.start_bar, 5);

    // Bar 0 does not exist on the 1-based wire.
    let response = call(
        &mut app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: id.into(),
            start_bar: 0,
        },
    );
    expect_error(response, ErrorKind::InvalidParams);

    // Overlapping the auto placement at bars 1-4 is a validation error.
    let response = call(
        &mut app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: id.into(),
            start_bar: 2,
        },
    );
    let message = expect_error(response, ErrorKind::InvalidParams);
    assert!(message.contains("overlap"), "unexpected message: {message}");

    let response = call(
        &mut app,
        "section.remove_placement",
        &section_proto::RemovePlacementParams {
            placement_id: placed.placement_id,
        },
    );
    let _: MutationAck = response.result().expect("remove succeeds");
    assert_eq!(sections_view(&mut app).placements.len(), 1);

    let response = call(
        &mut app,
        "section.remove_placement",
        &section_proto::RemovePlacementParams {
            placement_id: placed.placement_id,
        },
    );
    expect_error(response, ErrorKind::NotFound);
}

#[test]
fn set_scale_parses_wire_names() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let response = call(
        &mut app,
        "section.set_scale",
        &section_proto::SetScaleParams {
            section_id: id.into(),
            scale: key("F#", "harmonic_minor"),
        },
    );
    let _: MutationAck = response.result().expect("set_scale succeeds");

    let view = sections_view(&mut app);
    let scale = view.definitions[0].scale.as_ref().expect("scale set");
    assert_eq!(scale.tonic, "F#");
    assert_eq!(scale.scale, "harmonic minor");

    let response = call(
        &mut app,
        "section.set_scale",
        &section_proto::SetScaleParams {
            section_id: id.into(),
            scale: key("H", "minor"),
        },
    );
    expect_error(response, ErrorKind::InvalidParams);

    let response = call(
        &mut app,
        "section.set_scale",
        &section_proto::SetScaleParams {
            section_id: id.into(),
            scale: key("A", "klingon"),
        },
    );
    expect_error(response, ErrorKind::InvalidParams);
}

// ---------------- harmony.* ----------------

#[test]
fn add_chord_preserves_full_symbol() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id: id.into(),
            start_beat: 0.0,
            duration_beats: 4.0,
            symbol: "Am7".to_owned(),
        },
    );
    let added: harmony_proto::AddChordResult = response.result().expect("add succeeds");

    // A slash bass survives the trip — the control path carries the full
    // parsed chord, not just root + quality.
    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id: id.into(),
            start_beat: 4.0,
            duration_beats: 4.0,
            symbol: "C/E".to_owned(),
        },
    );
    let _: harmony_proto::AddChordResult = response.result().expect("slash chord adds");

    let view = sections_view(&mut app);
    assert_eq!(chord_symbols(&view, id), vec!["Am7", "C/E"]);
    assert_eq!(
        u64::from(view.definitions[0].chords[0].id),
        u64::from(added.chord_id)
    );
}

#[test]
fn add_chord_validates() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let ok = |app: &mut Resonance, start: f64, dur: f64, symbol: &str| {
        call(
            app,
            "harmony.add_chord",
            &harmony_proto::AddChordParams {
                section_id: id.into(),
                start_beat: start,
                duration_beats: dur,
                symbol: symbol.to_owned(),
            },
        )
    };

    let _: harmony_proto::AddChordResult =
        ok(&mut app, 0.0, 4.0, "C").result().expect("first add succeeds");

    // Overlap.
    let message = expect_error(ok(&mut app, 2.0, 4.0, "G"), ErrorKind::InvalidParams);
    assert!(message.contains("overlap"), "unexpected message: {message}");
    // Fractional grid position.
    expect_error(ok(&mut app, 4.5, 2.0, "G"), ErrorKind::InvalidParams);
    // Zero duration.
    expect_error(ok(&mut app, 4.0, 0.0, "G"), ErrorKind::InvalidParams);
    // Past the section end (4 bars * 4 beats).
    expect_error(ok(&mut app, 14.0, 4.0, "G"), ErrorKind::InvalidParams);
    // Unparseable symbol.
    let message = expect_error(ok(&mut app, 4.0, 4.0, "Xyz9"), ErrorKind::InvalidParams);
    assert!(message.contains("Xyz9"), "names the symbol: {message}");
    // Unknown section.
    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id: 999_999.into(),
            start_beat: 0.0,
            duration_beats: 4.0,
            symbol: "C".to_owned(),
        },
    );
    expect_error(response, ErrorKind::NotFound);

    // Nothing partial got applied along the way.
    assert_eq!(chord_symbols(&sections_view(&mut app), id), vec!["C"]);
}

#[test]
fn edit_chord_patches_in_one_transaction() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);
    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id: id.into(),
            start_beat: 0.0,
            duration_beats: 4.0,
            symbol: "C".to_owned(),
        },
    );
    let added: harmony_proto::AddChordResult = response.result().expect("add succeeds");

    let revision_before = added.revision;
    // Change symbol, position, and length in ONE call = one revision bump.
    let response = call(
        &mut app,
        "harmony.edit_chord",
        &harmony_proto::EditChordParams {
            section_id: id.into(),
            chord_id: added.chord_id,
            symbol: Some("Fmaj7".to_owned()),
            start_beat: Some(8.0),
            duration_beats: Some(8.0),
        },
    );
    let ack: MutationAck = response.result().expect("edit succeeds");
    assert_eq!(ack.revision, revision_before + 1);

    let view = sections_view(&mut app);
    let chord = &view.definitions[0].chords[0];
    assert_eq!(chord.symbol, "Fmaj7");
    assert_eq!(chord.start_beat, 8.0);
    assert_eq!(chord.duration_beats, 8.0);
    // The chord id stays stable across the edit.
    assert_eq!(chord.id, added.chord_id);

    let response = call(
        &mut app,
        "harmony.edit_chord",
        &harmony_proto::EditChordParams {
            section_id: id.into(),
            chord_id: 999_999.into(),
            symbol: Some("C".to_owned()),
            start_beat: None,
            duration_beats: None,
        },
    );
    expect_error(response, ErrorKind::NotFound);
}

#[test]
fn delete_chord_removes_only_that_chord() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);
    for (beat, symbol) in [(0.0, "C"), (4.0, "G")] {
        let response = call(
            &mut app,
            "harmony.add_chord",
            &harmony_proto::AddChordParams {
                section_id: id.into(),
                start_beat: beat,
                duration_beats: 4.0,
                symbol: symbol.to_owned(),
            },
        );
        assert!(response.error.is_none());
    }
    let first = sections_view(&mut app).definitions[0].chords[0].id;

    let response = call(
        &mut app,
        "harmony.delete_chord",
        &harmony_proto::DeleteChordParams {
            section_id: id.into(),
            chord_id: first,
        },
    );
    let _: MutationAck = response.result().expect("delete succeeds");
    assert_eq!(chord_symbols(&sections_view(&mut app), id), vec!["G"]);

    let response = call(
        &mut app,
        "harmony.delete_chord",
        &harmony_proto::DeleteChordParams {
            section_id: id.into(),
            chord_id: first,
        },
    );
    expect_error(response, ErrorKind::NotFound);
}

#[test]
fn apply_progression_symbols_replaces_grid_in_one_undo_step() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);
    // Pre-existing chord that the progression must replace.
    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id: id.into(),
            start_beat: 0.0,
            duration_beats: 4.0,
            symbol: "E7".to_owned(),
        },
    );
    let added: harmony_proto::AddChordResult = response.result().expect("add succeeds");

    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.symbols = Some(
        ["Am7", "Dm7", "G7", "Cmaj7"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    let response = call(&mut app, "harmony.apply_progression", &params);
    let result: harmony_proto::ApplyProgressionResult =
        response.result().expect("apply succeeds");
    assert_eq!(result.chord_ids.len(), 4);
    // One revision bump: the whole apply is a single undoable transaction.
    assert_eq!(result.revision, added.revision + 1);

    let view = sections_view(&mut app);
    assert_eq!(
        chord_symbols(&view, id),
        vec!["Am7", "Dm7", "G7", "Cmaj7"]
    );
    // Default layout: one bar (4 beats) per chord.
    let starts: Vec<f64> = view.definitions[0]
        .chords
        .iter()
        .map(|c| c.start_beat)
        .collect();
    assert_eq!(starts, vec![0.0, 4.0, 8.0, 12.0]);

    // A single undo restores the pre-apply grid.
    let _ = app.update(Message::Undo);
    assert_eq!(chord_symbols(&sections_view(&mut app), id), vec!["E7"]);
}

#[test]
fn apply_progression_numerals_render_diatonically() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.key = Some(key("A", "minor"));
    params.numerals = Some(
        ["i", "VI", "III", "VII"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    let response = call(&mut app, "harmony.apply_progression", &params);
    let _: harmony_proto::ApplyProgressionResult = response.result().expect("apply succeeds");

    assert_eq!(
        chord_symbols(&sections_view(&mut app), id),
        vec!["Am", "F", "C", "G"]
    );
}

#[test]
fn apply_progression_preset_and_options() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.key = Some(key("C", "major"));
    params.preset = Some("pop".to_owned());
    params.beats_per_chord = Some(2.0);
    let response = call(&mut app, "harmony.apply_progression", &params);
    let _: harmony_proto::ApplyProgressionResult = response.result().expect("apply succeeds");

    let view = sections_view(&mut app);
    assert_eq!(chord_symbols(&view, id), vec!["C", "G", "Am", "F"]);
    let starts: Vec<f64> = view.definitions[0]
        .chords
        .iter()
        .map(|c| c.start_beat)
        .collect();
    assert_eq!(starts, vec![0.0, 2.0, 4.0, 6.0]);

    // Sevenths flag renders diatonic sevenths.
    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.key = Some(key("C", "major"));
    params.numerals = Some(vec!["ii".to_owned(), "V".to_owned(), "I".to_owned()]);
    params.sevenths = Some(true);
    let response = call(&mut app, "harmony.apply_progression", &params);
    let _: harmony_proto::ApplyProgressionResult = response.result().expect("apply succeeds");
    assert_eq!(
        chord_symbols(&sections_view(&mut app), id),
        vec!["Dm7", "G7", "Cmaj7"]
    );
}

#[test]
fn apply_progression_validates_sources_and_fit() {
    let mut app = app_with_project();
    let id = create_section(&mut app, "Verse", 4);

    // No source at all.
    let params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    expect_error(
        call(&mut app, "harmony.apply_progression", &params),
        ErrorKind::InvalidParams,
    );

    // Two sources at once.
    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.symbols = Some(vec!["C".to_owned()]);
    params.preset = Some("pop".to_owned());
    expect_error(
        call(&mut app, "harmony.apply_progression", &params),
        ErrorKind::InvalidParams,
    );

    // Numerals without a key.
    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.numerals = Some(vec!["i".to_owned()]);
    expect_error(
        call(&mut app, "harmony.apply_progression", &params),
        ErrorKind::InvalidParams,
    );

    // Unknown preset names the known ones.
    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.key = Some(key("C", "major"));
    params.preset = Some("shoegaze".to_owned());
    let message = expect_error(
        call(&mut app, "harmony.apply_progression", &params),
        ErrorKind::InvalidParams,
    );
    assert!(message.contains("pop"), "lists known presets: {message}");

    // Invalid numeral.
    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.key = Some(key("C", "major"));
    params.numerals = Some(vec!["viii".to_owned()]);
    expect_error(
        call(&mut app, "harmony.apply_progression", &params),
        ErrorKind::InvalidParams,
    );

    // Too many chords for the section (4 bars * 4 beats = 16 beats).
    let mut params = harmony_proto::ApplyProgressionParams::for_section(id.into());
    params.symbols = Some(vec!["C".to_owned(); 5]);
    let message = expect_error(
        call(&mut app, "harmony.apply_progression", &params),
        ErrorKind::InvalidParams,
    );
    assert!(message.contains("16 beats"), "explains capacity: {message}");

    // Nothing was applied by any failed attempt.
    assert!(chord_symbols(&sections_view(&mut app), id).is_empty());
}
