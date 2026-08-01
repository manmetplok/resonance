//! Regressions for the three `generate.*` bugs found while driving a real
//! song through the control API.
//!
//! **Bug 1 (duplication after load).** A project whose drums track carried
//! one clip per section came back from disk with those clips *orphaned*:
//! `ComposeState::rebuild_derived_clips` only claimed a loaded clip when
//! the owning section had a `lane_generators` entry for its track, and
//! drum lanes have none — a section's drums come from its arrangement /
//! pattern bank instead. `materialize_drum_clips` therefore found nothing
//! to tear down and stacked a *second* clip on every section's bar. One
//! `generate.drums` call turned 11 clips into 22, and the orphaned
//! originals kept playing, so the whole song's drums doubled silently.
//!
//! **Bug 3 (unqueryable clip id).** `generate.*` returns a clip id
//! synchronously but only asked the engine to create the clip, whose
//! `MidiClipCreated` echo is asynchronous — so `song.notes(returned_id)`
//! failed for about a third of a second and every script needed a sleep.
//!
//! **Bug 4 (unrequested sections).** `generate.drums(section_id=X)`
//! re-materialised *every* section of the arrangement, writing a default
//! groove onto sections the caller never named.
//!
//! These tests drive the real control dispatch and the real
//! save→load replay, because bug 1 only manifests after a load.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song as song_proto;
use resonance_control::{Request, Response};

const DRUMS: u64 = 60;

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-generate-load.rprj"));
    // A fresh test app has no bar table until something rebuilds it, and
    // `bar_to_sample` then answers 0 for every bar — which would put every
    // section's clip on top of the next. The live app always has one.
    app.test_set_sample_rate(48_000);
    app.test_set_flat_tempo(120.0);
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

/// A named, auto-placed 4-bar section. Consecutive calls land on
/// consecutive free bars, so each section owns a distinct bar line.
fn section_named(app: &mut Resonance, name: &str) -> SectionDefinitionId {
    call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: name.to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id
}

fn generate_drums(app: &mut Resonance, section_id: SectionDefinitionId) -> GenerateResult {
    call(
        app,
        "generate.drums",
        &proto::DrumsParams {
            section_id,
            track_id: ProtoTrackId(DRUMS),
            pattern: Some("four-on-floor".to_owned()),
            density: None,
            seed: Some(11),
        },
    )
    .result::<GenerateResult>()
    .expect("generate.drums succeeds")
}

/// Start samples of every MIDI clip on the drums track, sorted. Two equal
/// entries means two clips stacked on the same bar — the audible symptom
/// of bug 1.
fn drum_clip_starts(app: &Resonance) -> Vec<u64> {
    let mut starts: Vec<u64> = app
        .test_midi_clips()
        .iter()
        .filter(|c| c.track_id == DRUMS)
        .map(|c| c.start_sample)
        .collect();
    starts.sort_unstable();
    starts
}

fn drum_clip_ids(app: &Resonance) -> Vec<u64> {
    let mut ids: Vec<u64> = app
        .test_midi_clips()
        .iter()
        .filter(|c| c.track_id == DRUMS)
        .map(|c| c.id)
        .collect();
    ids.sort_unstable();
    ids
}

// ---------------------------------------------------------------------------
// Bug 1 — the headline: generate after a load must replace, not duplicate.
// ---------------------------------------------------------------------------

#[test]
fn generate_drums_after_a_load_replaces_the_section_clip_instead_of_duplicating_it() {
    let mut app = app_with_project();
    let verse = section_named(&mut app, "Verse");
    let chorus = section_named(&mut app, "Chorus");
    let bridge = section_named(&mut app, "Bridge");
    app.test_add_drum_track(DRUMS);

    // Build the saved state: one drum clip per section, exactly as a song
    // driven through the API accumulates them.
    for section in [verse, chorus, bridge] {
        generate_drums(&mut app, section);
    }
    let starts_before = drum_clip_starts(&app);
    assert_eq!(starts_before.len(), 3, "one drum clip per section");
    assert_eq!(app.test_derived_clip_count(DRUMS), 3);

    // Save + reopen.
    let file = app.test_build_project_file();
    app.test_replay_loaded_project(file);

    assert_eq!(
        drum_clip_starts(&app),
        starts_before,
        "the three drum clips survive the load unchanged"
    );
    // The heart of the bug: the lane→clip association must be rehydrated
    // for drum tracks too, or the generator cannot find what to replace.
    assert_eq!(
        app.test_derived_clip_count(DRUMS),
        3,
        "derived_clips must be rehydrated for drum lanes on load"
    );

    // ONE generate call for ONE section.
    generate_drums(&mut app, verse);

    let starts_after = drum_clip_starts(&app);
    assert_eq!(
        starts_after, starts_before,
        "generating one section must leave three clips on three distinct bars, \
         not six stacked in pairs"
    );
    let mut deduped = starts_after.clone();
    deduped.dedup();
    assert_eq!(
        deduped, starts_after,
        "no two drum clips may share a start bar"
    );
}

/// The same guarantee for the *repeat* case the report measured: once the
/// count has been corrupted it stays corrupted, so a second and third call
/// must also hold the line.
#[test]
fn repeated_generates_after_a_load_never_grow_the_clip_count() {
    let mut app = app_with_project();
    let verse = section_named(&mut app, "Verse");
    let chorus = section_named(&mut app, "Chorus");
    app.test_add_drum_track(DRUMS);
    generate_drums(&mut app, verse);
    generate_drums(&mut app, chorus);

    let file = app.test_build_project_file();
    app.test_replay_loaded_project(file);

    for section in [verse, chorus, verse, chorus] {
        generate_drums(&mut app, section);
        assert_eq!(
            drum_clip_starts(&app).len(),
            2,
            "the drums track must stay at two clips"
        );
    }
}

// ---------------------------------------------------------------------------
// Bug 4 — only the requested section is touched.
// ---------------------------------------------------------------------------

#[test]
fn generate_drums_touches_only_the_requested_section() {
    let mut app = app_with_project();
    let verse = section_named(&mut app, "Verse");
    let chorus = section_named(&mut app, "Chorus");
    app.test_add_drum_track(DRUMS);

    // Only the verse has ever been generated for.
    let verse_result = generate_drums(&mut app, verse);
    assert_eq!(
        drum_clip_starts(&app).len(),
        1,
        "the chorus must NOT receive a default groove it never asked for"
    );
    assert_eq!(verse_result.clip_ids.len(), 1);

    // Now the chorus; the verse's clip must be left byte-identical (same
    // id — it was not even rewritten).
    let verse_clip = verse_result.clip_id.expect("the verse reported a clip");
    generate_drums(&mut app, chorus);
    assert_eq!(drum_clip_starts(&app).len(), 2, "one clip per generated section");
    assert!(
        drum_clip_ids(&app).contains(&u64::from(verse_clip)),
        "generating the chorus must not disturb the verse's clip"
    );
}

// ---------------------------------------------------------------------------
// Bug 3 — the returned clip id is queryable immediately.
// ---------------------------------------------------------------------------

#[test]
fn the_clip_id_generate_drums_returns_resolves_on_the_very_next_request() {
    let mut app = app_with_project();
    let section = section_named(&mut app, "Verse");
    app.test_add_drum_track(DRUMS);

    let clip_id = generate_drums(&mut app, section)
        .clip_id
        .expect("a clip was reported");

    // No engine echo is applied in between: this is the "same instant"
    // the report's `song.notes` failed at before the fix.
    let notes: song_proto::NotesView = call(
        &mut app,
        "song.notes",
        &song_proto::NotesParams {
            clip_id,
            range: None,
        },
    )
    .result()
    .expect("song.notes resolves the id generate.drums just returned");
    assert_eq!(notes.clip_id, clip_id);
    assert!(!notes.notes.is_empty(), "four-on-floor wrote notes");
}

#[test]
fn the_clip_id_generate_part_returns_resolves_on_the_very_next_request() {
    use resonance_control::methods::harmony as harmony_proto;
    use resonance_control::methods::generate::GenerateRole;
    use resonance_control::KeyScale;

    let mut app = app_with_project();
    let section = section_named(&mut app, "Verse");
    app.test_add_track(70, resonance_audio::types::TrackType::Instrument);

    let mut progression = harmony_proto::ApplyProgressionParams::for_section(section);
    progression.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    progression.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    call(&mut app, "harmony.apply_progression", &progression)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");

    let clip_id = call(
        &mut app,
        "generate.part",
        &proto::PartParams {
            section_id: section,
            track_id: ProtoTrackId(70),
            role: GenerateRole::Bass,
            chord_count: None,
            beats_per_chord: None,
            sevenths: None,
            seed: Some(5),
            options: None,
        },
    )
    .result::<GenerateResult>()
    .expect("generate.part succeeds")
    .clip_id
    .expect("a clip was reported");

    let notes: song_proto::NotesView = call(
        &mut app,
        "song.notes",
        &song_proto::NotesParams {
            clip_id,
            range: None,
        },
    )
    .result()
    .expect("song.notes resolves the id generate.part just returned");
    assert!(!notes.notes.is_empty(), "the bass generator wrote notes");
}

/// Regenerating a lane must not leave the replaced clip behind in the
/// app-side mirror — a stale entry would keep a deleted clip visible to
/// `song.*` and get re-serialized into the next save.
#[test]
fn regenerating_removes_the_replaced_clip_from_the_mirror() {
    let mut app = app_with_project();
    let section = section_named(&mut app, "Verse");
    app.test_add_drum_track(DRUMS);

    let first = generate_drums(&mut app, section)
        .clip_id
        .expect("a clip was reported");
    let second = generate_drums(&mut app, section)
        .clip_id
        .expect("a clip was reported");
    assert_ne!(first, second, "a regenerate issues a fresh clip");

    let ids = drum_clip_ids(&app);
    assert_eq!(ids, vec![u64::from(second)], "only the live clip remains");
}

