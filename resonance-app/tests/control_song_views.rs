//! `song.*` introspection views + the `plugins.catalog` query over the
//! control endpoint (ba doc #265, todo #1148).
//!
//! Builds a small project in-memory (tracks, clips, notes, a placed
//! section with chords, a vocal lane with a lyric draft, a song key)
//! and drives the read-only views through the real `update()` path,
//! asserting both the typed results and the raw JSON shapes (compact,
//! lowercase enums, real app ids).

use resonance_app::compose::{
    ChordState, GenerateParams, LaneGeneratorConfig, LaneGeneratorKind, SectionDefinitionState,
};
use resonance_app::chord_track::KeyChange;
use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{MidiClipState, PluginSlotState, ViewMode};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, MidiNote, ScannedPlugin, TrackType};
use resonance_control::methods::song::{
    NotesView, SectionsView, SongSummary, TracksView, VocalView,
};
use resonance_control::methods::plugins::PluginCatalog;
use resonance_control::{ErrorKind, Request, Response};
use resonance_music_theory::{
    parse_chord, LyricLine, Mode, MotifSource, PitchClass, Scale, VocalParams,
};

const SR: u32 = 48_000;
const INSTRUMENT: u64 = 1;
const VOCAL: u64 = 2;
const AUDIO: u64 = 3;
const MIDI_CLIP: u64 = 100;
const AUDIO_CLIP: u64 = 200;

fn request(id: i64, method: &str, params: serde_json::Value) -> Request {
    Request::new(id, method, &params).expect("params serialize")
}

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn section_definition(id: u64, name: &str, length_bars: u32) -> SectionDefinitionState {
    SectionDefinitionState {
        id,
        name: name.to_owned(),
        color: [0, 0, 0],
        length_bars,
        chords: Vec::new(),
        scale: Some(Scale::new(PitchClass::A, Mode::Minor)),
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    }
}

/// The shared fixture: 3 tracks (instrument w/ synth + fx, vocal with a
/// lyric draft, audio with one clip), one MIDI clip with 3 notes, one
/// placed 2-bar section with an Am7 chord, an A-minor song key, and a
/// project dictionary entry.
fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(SR);
    app.test_rebuild_tempo_map();
    app.test_set_active_project(true);

    app.test_add_track(INSTRUMENT, TrackType::Instrument);
    app.test_add_track(VOCAL, TrackType::Vocal);
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_push_track_plugin(
        INSTRUMENT,
        PluginSlotState::new(
            900,
            "Test Synth".to_owned(),
            "com.test.synth".to_owned(),
            "/plugins/test.clap".to_owned(),
            Vec::new(),
            false,
        ),
    );
    app.test_push_track_plugin(
        INSTRUMENT,
        PluginSlotState::new(
            901,
            "Test EQ".to_owned(),
            "com.test.eq".to_owned(),
            "/plugins/eq.clap".to_owned(),
            Vec::new(),
            false,
        ),
    );

    app.test_push_midi_clip(MidiClipState {
        id: MIDI_CLIP,
        track_id: INSTRUMENT,
        start_sample: 0,
        duration_ticks: 4 * 480,
        name: "riff".to_owned(),
        notes: vec![
            MidiNote {
                note: 60,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 480,
            },
            MidiNote {
                note: 64,
                velocity: 0.5,
                start_tick: 480,
                duration_ticks: 480,
            },
            MidiNote {
                note: 72,
                velocity: 1.0,
                start_tick: 960,
                duration_ticks: 960,
            },
        ],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app.test_push_clip(resonance_app::state::ClipState {
        id: AUDIO_CLIP,
        track_id: AUDIO,
        start_sample: 0,
        duration_samples: 2 * SR as u64,
        name: "take".to_owned(),
        total_frames: 2 * SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: Default::default(),
        fade_out_frames: 0,
        fade_out_curve: Default::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    });

    let mut def = section_definition(1, "Verse", 2);
    def.chords.push(ChordState {
        id: 10,
        start_beat: 0,
        duration_beats: 4,
        chord: parse_chord("Am7").expect("valid chord"),
    });
    def.lane_generators.insert(
        VOCAL,
        LaneGeneratorConfig {
            kind: LaneGeneratorKind::Vocal(VocalParams {
                draft: vec![LyricLine {
                    n: 1,
                    rhyme: 'A',
                    syllables: 2,
                    text: "hello".to_owned(),
                    locked: false,
                }],
                ..VocalParams::default()
            }),
            seed: 0,
        },
    );
    app.test_push_section_definition(def);
    let _ = app.test_place_section(1, 0);

    app.test_chord_track_mut().insert_key_change(KeyChange {
        id: 1,
        start_sample: 0,
        scale: Scale::new(PitchClass::A, Mode::Minor),
    });
    app.test_push_dictionary_entry("hello", vec!["hh", "ax", "l", "ow"]);
    app
}

// ---------------- song.summary ----------------

#[test]
fn summary_reports_song_and_tracks() {
    let mut app = app();
    let response = roundtrip(&mut app, Request::without_params(1, "song.summary"));
    let summary: SongSummary = response.result().expect("summary succeeds");

    assert_eq!(summary.tempo_bpm, 120.0);
    assert_eq!(summary.time_signature.numerator, 4);
    assert_eq!(summary.time_signature.denominator, 4);
    assert_eq!(summary.sample_rate, SR);
    let key = summary.key.expect("song key set");
    assert_eq!(key.tonic, "A");
    assert_eq!(key.scale, "minor");
    assert_eq!(summary.playhead.bar, 1);
    assert!(summary.playhead.beat >= 1.0);
    // The 2-bar section is the furthest content: 4s at 120 bpm 4/4.
    assert_eq!(summary.length_samples, 4 * SR as u64);
    assert!((summary.length_bars - 2.0).abs() < 1e-6);

    assert_eq!(summary.sections.len(), 1);
    assert_eq!(summary.sections[0].name, "Verse");
    assert_eq!(summary.sections[0].start_bar, 1);
    assert_eq!(summary.sections[0].length_bars, 2);

    let by_id = |raw: u64| {
        summary
            .tracks
            .iter()
            .find(|t| u64::from(t.id) == raw)
            .expect("track present")
    };
    assert_eq!(by_id(INSTRUMENT).instrument.as_deref(), Some("com.test.synth"));
    assert_eq!(by_id(INSTRUMENT).clip_count, 1);
    assert_eq!(by_id(AUDIO).clip_count, 1);
    assert_eq!(by_id(VOCAL).clip_count, 0);
    // Unity fader (0 dB) is 1.0 linear on the wire.
    assert!((by_id(AUDIO).volume - 1.0).abs() < 1e-6);
    assert_eq!(summary.revision, app.revision());
}

#[test]
fn summary_json_uses_lowercase_enums_and_compact_shape() {
    let mut app = app();
    let response = roundtrip(&mut app, Request::without_params(1, "song.summary"));
    let json: serde_json::Value = response.result().expect("summary succeeds");

    assert_eq!(json["transport"], "stopped");
    assert_eq!(json["key"]["scale"], "minor");
    let tracks = json["tracks"].as_array().expect("tracks array");
    let kind_of = |raw: u64| {
        tracks
            .iter()
            .find(|t| t["id"] == serde_json::json!(raw))
            .expect("track in json")["kind"]
            .clone()
    };
    assert_eq!(kind_of(INSTRUMENT), "instrument");
    assert_eq!(kind_of(VOCAL), "vocal");
    assert_eq!(kind_of(AUDIO), "audio");
    // Ids are plain JSON numbers (real app ids), not wrapped objects.
    assert!(json["sections"][0]["id"].is_u64());
    // No UI/view state leaks into the view.
    assert!(json.get("view_mode").is_none());
    assert!(json.get("scroll_y").is_none());
}

// ---------------- song.sections ----------------

#[test]
fn sections_reports_definitions_chords_and_placements() {
    let mut app = app();
    let response = roundtrip(&mut app, Request::without_params(1, "song.sections"));
    let sections: SectionsView = response.result().expect("sections succeeds");

    assert_eq!(sections.definitions.len(), 1);
    let def = &sections.definitions[0];
    assert_eq!(def.name, "Verse");
    assert_eq!(def.length_bars, 2);
    assert_eq!(def.scale.as_ref().map(|s| s.scale.as_str()), Some("minor"));
    assert_eq!(def.chords.len(), 1);
    let chord = &def.chords[0];
    assert_eq!(u64::from(chord.id), 10);
    assert_eq!(chord.start_beat, 0.0);
    assert_eq!(chord.duration_beats, 4.0);
    assert_eq!(chord.symbol, parse_chord("Am7").unwrap().to_string());

    assert_eq!(sections.placements.len(), 1);
    assert_eq!(u64::from(sections.placements[0].definition_id), 1);
    assert_eq!(sections.placements[0].start_bar, 1);
}

// ---------------- song.tracks ----------------

#[test]
fn tracks_reports_clips_and_effect_chain() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "song.tracks", serde_json::json!({ "track_id": INSTRUMENT })),
    );
    let view: TracksView = response.result().expect("tracks succeeds");
    assert_eq!(view.tracks.len(), 1);
    let track = &view.tracks[0];
    assert_eq!(track.summary.instrument.as_deref(), Some("com.test.synth"));
    // Slot 0 is the instrument; the chain reports only inserts.
    assert_eq!(track.effects, vec!["com.test.eq".to_owned()]);
    assert_eq!(track.clips.len(), 1);
    let clip = &track.clips[0];
    assert_eq!(u64::from(clip.id), MIDI_CLIP);
    assert!(clip.midi);
    assert_eq!(clip.length_beats, 4.0);
    assert_eq!(clip.start.bar, 1);

    // Audio clip lengths come back in both units.
    let response = roundtrip(
        &mut app,
        request(2, "song.tracks", serde_json::json!({ "track_id": AUDIO })),
    );
    let view: TracksView = response.result().expect("tracks succeeds");
    let clip = &view.tracks[0].clips[0];
    assert!(!clip.midi);
    assert_eq!(clip.length_samples, 2 * SR as u64);
    assert!((clip.length_beats - 4.0).abs() < 1e-6); // 2s at 120bpm

    // Omitting track_id returns every track.
    let response = roundtrip(&mut app, Request::without_params(3, "song.tracks"));
    let view: TracksView = response.result().expect("tracks succeeds");
    assert_eq!(view.tracks.len(), 3);
}

#[test]
fn tracks_reports_frozen_state() {
    use resonance_app::state::FreezeStatus;
    use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

    let mut app = app();
    let cache = FreezeCacheRef::new(
        "freeze_1.wav".to_owned(),
        SR,
        32,
        1,
        FreezeCacheStatus::Frozen,
    );
    app.test_set_freeze_status(INSTRUMENT, FreezeStatus::Frozen { cache_ref: cache.clone() });

    let response = roundtrip(&mut app, Request::without_params(1, "song.tracks"));
    let view: TracksView = response.result().expect("tracks succeeds");
    let by_id = |raw: u64| {
        view.tracks
            .iter()
            .find(|t| u64::from(t.summary.id) == raw)
            .expect("track present")
    };
    assert!(by_id(INSTRUMENT).frozen, "frozen cache -> frozen on the wire");
    assert!(!by_id(AUDIO).frozen, "unfrozen tracks report false");

    // Stale (inputs drifted, cache still attached) also rejects edits
    // (#576), so it reads as frozen too.
    app.test_set_freeze_status(INSTRUMENT, FreezeStatus::Stale { cache_ref: cache });
    let response = roundtrip(&mut app, Request::without_params(2, "song.tracks"));
    let view: TracksView = response.result().expect("tracks succeeds");
    assert!(by_id_of(&view, INSTRUMENT).frozen);

    // A mid-render track has no cache yet -> not frozen on the wire.
    app.test_set_freeze_status(INSTRUMENT, FreezeStatus::Freezing { fraction: 0.5 });
    let response = roundtrip(&mut app, Request::without_params(3, "song.tracks"));
    let view: TracksView = response.result().expect("tracks succeeds");
    assert!(!by_id_of(&view, INSTRUMENT).frozen);

    // The JSON stays compact: `frozen` is omitted entirely when false.
    let json: serde_json::Value = roundtrip(&mut app, Request::without_params(4, "song.tracks"))
        .result()
        .expect("tracks succeeds");
    let audio = json["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == serde_json::json!(AUDIO))
        .unwrap();
    assert!(audio.get("frozen").is_none());
}

fn by_id_of(view: &TracksView, raw: u64) -> &resonance_control::methods::song::TrackDetail {
    view.tracks
        .iter()
        .find(|t| u64::from(t.summary.id) == raw)
        .expect("track present")
}

#[test]
fn tracks_unknown_id_is_not_found() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "song.tracks", serde_json::json!({ "track_id": 9999 })),
    );
    assert_eq!(
        response.error.expect("unknown track rejected").kind(),
        ErrorKind::NotFound
    );
}

// ---------------- song.notes ----------------

#[test]
fn notes_reports_pitch_names_ticks_and_beats() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "song.notes", serde_json::json!({ "clip_id": MIDI_CLIP })),
    );
    let view: NotesView = response.result().expect("notes succeeds");
    assert_eq!(view.notes.len(), 3);
    let first = &view.notes[0];
    assert_eq!(first.index, 0);
    assert_eq!(first.pitch, 60);
    assert_eq!(first.pitch_name, "C4");
    assert_eq!(first.start_tick, 0);
    assert_eq!(first.start_beat, 0.0);
    assert_eq!(first.duration_beats, 1.0);
    assert_eq!(first.velocity, 102); // 0.8 * 127, rounded
    assert_eq!(view.notes[2].pitch_name, "C5");
    assert_eq!(view.notes[2].velocity, 127);
}

#[test]
fn notes_range_filters_by_overlap() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(
            1,
            "song.notes",
            serde_json::json!({
                "clip_id": MIDI_CLIP,
                "range": { "start_beat": 1.0, "end_beat": 2.0 }
            }),
        ),
    );
    let view: NotesView = response.result().expect("notes succeeds");
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].pitch, 64);
    // The original note index survives filtering.
    assert_eq!(view.notes[0].index, 1);
}

#[test]
fn notes_rejects_audio_and_unknown_clips() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "song.notes", serde_json::json!({ "clip_id": AUDIO_CLIP })),
    );
    let error = response.error.expect("audio clip rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("audio clip"));

    let response = roundtrip(
        &mut app,
        request(2, "song.notes", serde_json::json!({ "clip_id": 424242 })),
    );
    assert_eq!(response.error.expect("unknown clip").kind(), ErrorKind::NotFound);
}

// ---------------- song.vocal ----------------

#[test]
fn vocal_reports_lines_phonemes_and_dictionary() {
    let mut app = app();
    // No track_id: the first vocal track is picked.
    let response = roundtrip(&mut app, Request::without_params(1, "song.vocal"));
    let view: VocalView = response.result().expect("vocal succeeds");
    assert_eq!(u64::from(view.track_id), VOCAL);

    assert_eq!(view.lines.len(), 1);
    let line = &view.lines[0];
    assert_eq!(line.index, 0);
    assert_eq!(line.text, "hello");
    assert!(!line.syllables.is_empty());
    // The project dictionary overrides the CMU transcription, so the
    // seeded phonemes come back through the resolver...
    let phonemes: Vec<&str> = line
        .syllables
        .iter()
        .flat_map(|s| s.phonemes.iter().map(String::as_str))
        .collect();
    assert_eq!(phonemes, vec!["hh", "ax", "l", "ow"]);
    // ...and verbatim in the overrides map.
    assert_eq!(
        view.pronunciation_overrides.get("hello"),
        Some(&vec![
            "hh".to_owned(),
            "ax".to_owned(),
            "l".to_owned(),
            "ow".to_owned()
        ])
    );

    // Nothing rendered in this fixture.
    let json: serde_json::Value =
        roundtrip(&mut app, Request::without_params(2, "song.vocal"))
            .result()
            .expect("vocal succeeds");
    assert_eq!(json["render_state"], "not_rendered");
}

#[test]
fn vocal_track_resolution_errors() {
    let mut app = app();
    let response = roundtrip(
        &mut app,
        request(1, "song.vocal", serde_json::json!({ "track_id": AUDIO })),
    );
    assert_eq!(
        response.error.expect("non-vocal rejected").kind(),
        ErrorKind::InvalidParams
    );
    let response = roundtrip(
        &mut app,
        request(2, "song.vocal", serde_json::json!({ "track_id": 9999 })),
    );
    assert_eq!(response.error.expect("unknown track").kind(), ErrorKind::NotFound);
}

// ---------------- plugins.catalog ----------------

#[test]
fn plugin_catalog_lists_scanned_plugins() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
        ],
    });

    let response = roundtrip(&mut app, Request::without_params(1, "plugins.catalog"));
    let catalog: PluginCatalog = response.result().expect("catalog succeeds");
    assert_eq!(catalog.plugins.len(), 2);
    assert_eq!(catalog.plugins[0].id, "com.resonance.wavetable");

    let json: serde_json::Value = roundtrip(&mut app, Request::without_params(2, "plugins.catalog"))
        .result()
        .expect("catalog succeeds");
    assert_eq!(json["plugins"][0]["kind"], "instrument");
    assert_eq!(json["plugins"][1]["kind"], "effect");
}

// ---------------- read-only guarantee ----------------

#[test]
fn song_views_never_mutate_or_bump_revision() {
    let mut app = app();
    let before = app.revision();
    for method in [
        "song.summary",
        "song.sections",
        "song.tracks",
        "song.vocal",
        "plugins.catalog",
    ] {
        let response = roundtrip(&mut app, Request::without_params(1, method));
        assert!(response.error.is_none(), "{method} should succeed");
    }
    let _ = roundtrip(
        &mut app,
        request(2, "song.notes", serde_json::json!({ "clip_id": MIDI_CLIP })),
    );
    assert_eq!(app.revision(), before);
    assert!(!app.is_dirty());
}
