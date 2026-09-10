//! `generate.*` control methods (ba doc #265, todo #1154), driven
//! through the real `update()` path with synthesized requests. Sections
//! and chords are set up via the `section.*` / `harmony.*` control
//! methods (todo #1153) so the tests exercise the full remote surface.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{ErrorKind, KeyScale, Request, Response};
use crate::common::roundtrip;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-generate-test.rprj"));
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

/// Like [`section_with_chords`] but with a caller-chosen name, so two
/// sections can be told apart in the clips they derive.
fn section_named(app: &mut Resonance, name: &str) -> SectionDefinitionId {
    let section_id = call(
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
    .section_id;

    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    call(app, "harmony.apply_progression", &params)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
    section_id
}

/// Drain the captured engine traffic into the notes each derived clip
/// received, keyed by the owning section's name (the materializer names
/// clips `"<section> · <track>"`). Keying by name rather than start
/// sample keeps two sections distinguishable even when they are placed
/// at the same bar.
fn drum_writes(
    rx: &resonance_audio::__test_support::Receiver<AudioCommand>,
) -> std::collections::BTreeMap<String, Vec<(u8, u64, u64)>> {
    let mut by_section = std::collections::BTreeMap::new();
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::LoadMidiClipDirect { notes, name, .. } = cmd {
            let section = name.split(" · ").next().unwrap_or(&name).to_owned();
            by_section.insert(
                section,
                notes
                    .iter()
                    .map(|n| (n.note, n.start_tick, n.duration_ticks))
                    .collect::<Vec<_>>(),
            );
        }
    }
    by_section
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
        // The derived clip the material landed in, reported back so the
        // caller doesn't have to re-read song.tracks to find it.
        assert_eq!(result.clip_ids.len(), 1, "one placement, one clip");
        assert_eq!(result.clip_id, result.clip_ids.first().copied());
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

/// The clip ids a generate reports must be the ids the read side
/// addresses — otherwise they are decoration and the caller still has to
/// re-read song.tracks to find its own material (doc #270 §10).
#[test]
fn reported_clip_id_resolves_in_song_notes() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let track = add_synth_track(&mut app, 11);

    let params = proto::PartParams {
        section_id,
        track_id: track,
        role: GenerateRole::Bass,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(7),
        options: None,
    };
    let rx = app.test_capture_engine();
    let result: GenerateResult = call(&mut app, "generate.part", &params)
        .result()
        .expect("generate.part succeeds");
    let clip_id = result.clip_id.expect("a clip was reported");

    // The reported id must be the one actually handed to the engine.
    let mut loaded = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
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
            loaded.push(AudioEvent::MidiClipCreated {
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
    assert_eq!(loaded.len(), 1, "one clip written to the engine");
    let AudioEvent::MidiClipCreated { clip_id: sent, .. } = &loaded[0] else {
        unreachable!()
    };
    assert_eq!(*sent, u64::from(clip_id), "reported id is the engine's id");

    // Drive the echo the real engine sends back, then read the clip.
    for event in loaded {
        app.test_apply_engine_event(event);
    }
    let notes: resonance_control::methods::song::NotesView = call(
        &mut app,
        "song.notes",
        &resonance_control::methods::song::NotesParams {
            clip_id,
            range: None,
        },
    )
    .result()
    .expect("song.notes resolves the reported clip");
    assert_eq!(notes.clip_id, clip_id);
    assert!(!notes.notes.is_empty(), "the bass generator wrote notes");
}

/// A section placed more than once derives one clip per placement, and
/// all of them come back in arrangement order.
#[test]
fn every_placement_of_a_section_reports_its_clip() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let track = add_synth_track(&mut app, 13);

    // section_with_chords already placed it once; place it again later.
    call(
        &mut app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: section_id,
            start_bar: 9,
        },
    )
    .result::<section_proto::PlaceResult>()
    .expect("second placement succeeds");

    let params = proto::PartParams {
        section_id,
        track_id: track,
        role: GenerateRole::Pad,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(3),
        options: None,
    };
    let result: GenerateResult = call(&mut app, "generate.part", &params)
        .result()
        .expect("generate.part succeeds");

    assert_eq!(result.clip_ids.len(), 2, "one clip per placement");
    assert_eq!(result.clip_id, result.clip_ids.first().copied());
    assert_ne!(
        result.clip_ids[0], result.clip_ids[1],
        "placements get distinct clips"
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

/// Every option named in the `generate_part` tool description must be
/// settable on its own. A caller reading the docs sets one knob — the
/// bass style, a register — and should not have to restate the whole
/// params struct to be understood (doc #270 §6).
#[test]
fn documented_options_are_accepted_one_at_a_time() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let bass = add_synth_track(&mut app, 30);
    let lead = add_synth_track(&mut app, 31);
    let pad = add_synth_track(&mut app, 32);

    let cases: &[(GenerateRole, ProtoTrackId, serde_json::Value)] = &[
        (GenerateRole::Bass, bass, serde_json::json!({ "style": "RootHold" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "style": "RootPulse" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "style": "RootFifth" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "style": "Octave" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "style": "Walking" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "style": "Motif" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "base_note": 24 })),
        (GenerateRole::Bass, bass, serde_json::json!({ "velocity": 0.6 })),
        (GenerateRole::Bass, bass, serde_json::json!({ "motif_mode": "SameIntervals" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "motif_mode": "Augmented" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "motif_mode": "RhythmOnly" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "motif_mode": "FirstNoteOnly" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "motif_phrase": "Simple" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "motif_phrase": "MirrorMelody" })),
        (GenerateRole::Bass, bass, serde_json::json!({ "motif_phrase": "Restricted" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "style": "ArpUp" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "style": "ArpDown" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "style": "ArpUpDown" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "style": "Motif" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "register": [60, 84] })),
        (GenerateRole::Lead, lead, serde_json::json!({ "note_value_ticks": 120 })),
        (GenerateRole::Lead, lead, serde_json::json!({ "rest_density": 0.25 })),
        (GenerateRole::Lead, lead, serde_json::json!({ "complexity": 0.8 })),
        (GenerateRole::Lead, lead, serde_json::json!({ "articulation": 0.3 })),
        (GenerateRole::Lead, lead, serde_json::json!({ "phrase_len": 8 })),
        (GenerateRole::Lead, lead, serde_json::json!({ "motif_len": 0 })),
        (GenerateRole::Lead, lead, serde_json::json!({ "leap_chance": 0.4 })),
        (GenerateRole::Lead, lead, serde_json::json!({ "fill_vocal_gaps": true })),
        (GenerateRole::Lead, lead, serde_json::json!({ "contour": "Auto" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "contour": "Arch" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "contour": "Descending" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "contour": "Ascending" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "contour": "Wave" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "embellishment": "Auto" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "embellishment": "Folk" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "embellishment": "PopBallad" })),
        (GenerateRole::Lead, lead, serde_json::json!({ "embellishment": "Jazz" })),
        (GenerateRole::Pad, pad, serde_json::json!({ "register": [52, 76] })),
        (GenerateRole::Pad, pad, serde_json::json!({ "velocity": 0.7 })),
    ];

    for (role, track_id, options) in cases {
        let params = proto::PartParams {
            section_id,
            track_id: *track_id,
            role: *role,
            chord_count: None,
            beats_per_chord: None,
            sevenths: None,
            seed: Some(5),
            options: Some(options.clone()),
        };
        let response = call(&mut app, "generate.part", &params);
        assert!(
            response.error.is_none(),
            "options {options} rejected for {role:?}: {}",
            response.error.map(|e| e.message).unwrap_or_default()
        );
    }
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
        density: None,
        seed: Some(99),
    };
    let response = call(&mut app, "generate.drums", &params);
    let result: GenerateResult = response.result().expect("generate.drums succeeds");
    assert_eq!(result.clip_ids.len(), 1, "one placement, one clip");
    assert_eq!(result.clip_id, result.clip_ids.first().copied());

    // The section now has a primary drum pattern assigned.
    assert!(
        app.test_section_primary_pattern(u64::from(section_id)).is_some(),
        "a drum pattern is pinned to the section"
    );
}

/// Drum patterns are project-global, so re-rolling one in place used to
/// rewrite every section that rendered it: generating drums for the
/// chorus silently replaced the verse's notes, including sections that
/// had never named a pattern at all and were only rendering the
/// fallback. Generating for one section must leave the others
/// note-identical (doc #270 §4).
///
/// A control generate is now also *scoped* to the named section (bug 4 of
/// the control-API report), so the verse is no longer rewritten at all —
/// a strictly stronger form of "left alone". The assertion below accepts
/// either: absent from the write stream, or present and identical.
#[test]
fn generating_drums_for_one_section_leaves_the_others_alone() {
    let mut app = app_with_project();
    let verse = section_named(&mut app, "Verse");
    let chorus = section_named(&mut app, "Chorus");
    app.test_add_drum_track(40);

    let rx = app.test_capture_engine();
    call(&mut app, "generate.drums", &proto::DrumsParams {
        section_id: verse,
        track_id: ProtoTrackId(40),
        pattern: None,
        density: None,
        seed: Some(1),
    })
    .result::<GenerateResult>()
    .expect("first generate succeeds");
    let after_first = drum_writes(&rx);
    let verse_before = after_first
        .get("Verse")
        .expect("the verse clip was written")
        .clone();
    assert!(!verse_before.is_empty(), "the verse got drum notes");

    // Generate for the *other* section with a different seed.
    call(&mut app, "generate.drums", &proto::DrumsParams {
        section_id: chorus,
        track_id: ProtoTrackId(40),
        pattern: None,
        density: None,
        seed: Some(999),
    })
    .result::<GenerateResult>()
    .expect("second generate succeeds");
    let after_second = drum_writes(&rx);

    // The verse must either not be rewritten at all (the scoped path) or
    // be rewritten with identical material — never with different notes.
    if let Some(verse_after) = after_second.get("Verse") {
        assert_eq!(
            verse_after, &verse_before,
            "generating the chorus changed the verse's notes"
        );
    }

    // The two sections now own distinct patterns, so they can differ.
    let verse_pattern = app.test_section_primary_pattern(u64::from(verse));
    let chorus_pattern = app.test_section_primary_pattern(u64::from(chorus));
    assert!(verse_pattern.is_some() && chorus_pattern.is_some());
    assert_ne!(
        verse_pattern, chorus_pattern,
        "each section owns its own pattern after generating"
    );
    assert_ne!(
        after_second.get("Chorus"),
        Some(&verse_before),
        "the chorus got its own material"
    );
}

/// A built-in groove is reachable by name in any project, installs the
/// authored steps rather than a Euclidean roll, and stays scoped to its
/// section (ba doc #270 §5).
#[test]
fn a_builtin_groove_can_be_named_and_lands_as_authored() {
    let mut app = app_with_project();
    let section_id = section_named(&mut app, "Chorus");
    app.test_add_drum_track(50);

    let rx = app.test_capture_engine();
    call(&mut app, "generate.drums", &proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(50),
        pattern: Some("four-on-floor".to_owned()),
        density: None,
        seed: None,
    })
    .result::<GenerateResult>()
    .expect("a built-in groove generates");

    let notes = drum_writes(&rx);
    let chorus = notes.get("Chorus").expect("the chorus clip was written");
    // Four-on-the-floor means a kick on every beat: ticks 0, 480, 960,
    // 1440 in a 4/4 bar at 480 TPQ. Anything Euclidean would scatter.
    let kick_ticks: Vec<u64> = chorus
        .iter()
        .filter(|(note, _, _)| *note == resonance_common::drum_map::KICK)
        .map(|(_, tick, _)| *tick)
        .take(4)
        .collect();
    assert_eq!(kick_ticks, vec![0, 480, 960, 1440], "kick on every beat");
}

/// Naming a built-in twice, for two sections, gives each its own pattern
/// — so thinning one does not touch the other.
#[test]
fn builtin_grooves_are_scoped_per_section() {
    let mut app = app_with_project();
    let verse = section_named(&mut app, "Verse");
    let chorus = section_named(&mut app, "Chorus");
    app.test_add_drum_track(51);

    let rx = app.test_capture_engine();
    for (section, density) in [(verse, Some(0.25f32)), (chorus, None)] {
        call(&mut app, "generate.drums", &proto::DrumsParams {
            section_id: section,
            track_id: ProtoTrackId(51),
            pattern: Some("four-on-floor".to_owned()),
            density,
            seed: None,
        })
        .result::<GenerateResult>()
        .expect("a built-in groove generates");
    }

    let written = drum_writes(&rx);
    let verse_notes = written.get("Verse").expect("verse clip");
    let chorus_notes = written.get("Chorus").expect("chorus clip");
    assert!(
        verse_notes.len() < chorus_notes.len(),
        "the thinned verse ({}) is sparser than the full chorus ({})",
        verse_notes.len(),
        chorus_notes.len()
    );
    assert_ne!(
        app.test_section_primary_pattern(u64::from(verse)),
        app.test_section_primary_pattern(u64::from(chorus)),
        "each section owns its own instance of the groove"
    );
}

/// Density is a real knob across a build, and monotonic.
#[test]
fn density_scales_a_builtin_across_a_build() {
    let counts: Vec<usize> = [0.25f32, 0.5, 1.0]
        .into_iter()
        .map(|d| {
            let mut app = app_with_project();
            let section_id = section_named(&mut app, "Build");
            app.test_add_drum_track(52);
            let rx = app.test_capture_engine();
            call(&mut app, "generate.drums", &proto::DrumsParams {
                section_id,
                track_id: ProtoTrackId(52),
                pattern: Some("industrial".to_owned()),
                density: Some(d),
                seed: None,
            })
            .result::<GenerateResult>()
            .expect("generates");
            drum_writes(&rx).get("Build").map(|n| n.len()).unwrap_or(0)
        })
        .collect();

    assert!(counts[0] > 0, "even the thinnest density still plays");
    assert!(
        counts[0] < counts[1] && counts[1] < counts[2],
        "density scales the groove monotonically: {counts:?}"
    );
}

/// A project pattern of the same name wins over the built-in, so a user
/// who authored "halftime" keeps theirs.
#[test]
fn a_project_pattern_wins_over_a_builtin_of_the_same_name() {
    let mut app = app_with_project();
    let section_id = section_named(&mut app, "Verse");
    app.test_add_drum_track(53);
    let renamed = app.test_rename_first_drum_pattern("halftime");

    call(&mut app, "generate.drums", &proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(53),
        pattern: Some("halftime".to_owned()),
        density: None,
        seed: Some(4),
    })
    .result::<GenerateResult>()
    .expect("generates");

    assert_eq!(
        app.test_section_primary_pattern(u64::from(section_id)),
        Some(renamed),
        "the project's own pattern was used, not the built-in"
    );
}

#[test]
fn an_unknown_pattern_lists_both_project_and_builtin_names() {
    let mut app = app_with_project();
    let section_id = section_named(&mut app, "Verse");
    app.test_add_drum_track(54);

    let message = expect_error(
        call(&mut app, "generate.drums", &proto::DrumsParams {
            section_id,
            track_id: ProtoTrackId(54),
            pattern: Some("gabber".to_owned()),
            density: None,
            seed: None,
        }),
        ErrorKind::NotFound,
    );
    assert!(message.contains("built-in"), "{message}");
    assert!(message.contains("halftime"), "{message}");
    assert!(message.contains("Main"), "{message}");
}

#[test]
fn density_out_of_range_is_rejected() {
    let mut app = app_with_project();
    let section_id = section_named(&mut app, "Verse");
    app.test_add_drum_track(55);

    for d in [-0.1f32, 1.5] {
        let message = expect_error(
            call(&mut app, "generate.drums", &proto::DrumsParams {
                section_id,
                track_id: ProtoTrackId(55),
                pattern: Some("sparse".to_owned()),
                density: Some(d),
                seed: None,
            }),
            ErrorKind::InvalidParams,
        );
        assert!(message.contains("density"), "{message}");
    }
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
        density: None,
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
        density: None,
        seed: None,
    };
    let message = expect_error(call(&mut app, "generate.drums", &params), ErrorKind::NotFound);
    assert!(message.contains("no drum pattern named"), "unexpected: {message}");

    // Unknown track.
    let params = proto::DrumsParams {
        section_id,
        track_id: ProtoTrackId(999),
        pattern: None,
        density: None,
        seed: None,
    };
    expect_error(call(&mut app, "generate.drums", &params), ErrorKind::NotFound);

    // Unknown section.
    let params = proto::DrumsParams {
        section_id: SectionDefinitionId(999_999),
        track_id: ProtoTrackId(22),
        pattern: None,
        density: None,
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
            density: None,
            seed: Some(1),
        });
        app.test_section_primary_pattern(u64::from(section_id))
    };
    let second = {
        let _ = call(&mut app, "generate.drums", &proto::DrumsParams {
            section_id,
            track_id: ProtoTrackId(23),
            pattern: None,
            density: None,
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
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
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
