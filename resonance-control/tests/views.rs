//! Compact song views: exact documented JSON shapes (lowercase enums,
//! musical + sample positions, real app ids) and round-trips.

use resonance_control::common::{
    KeyScale, SongPosition, TimeSignature, TrackKind, TrackOutput, TransportState,
};
use resonance_control::ids::{ChordId, ClipId, SectionDefinitionId, SectionPlacementId, TrackId};
use resonance_control::methods::song::{
    ChordView, ClipView, LyricLineView, NoteView, NotesView, SectionDefinitionView,
    SectionPlacementView, SectionsView, SongSummary, SyllableView, TrackDetail, TrackSummary,
    PitchRangeView, TracksView, VocalLaneView, VocalNoteView, VocalRenderState, VocalView,
};
use serde_json::json;
use std::collections::BTreeMap;

fn track_summary() -> TrackSummary {
    TrackSummary {
        id: TrackId(4),
        name: "Bass".to_owned(),
        kind: TrackKind::Instrument,
        instrument: Some("resonance-wavetable".to_owned()),
        parent_id: None,
        muted: false,
        soloed: true,
        // Exactly representable in f32 so the JSON shape check below
        // stays bit-stable through the f32 -> f64 widening.
        volume: 0.5,
        volume_db: -6.0,
        pan: -0.25,
        output: TrackOutput::Master,
        clip_count: 2,
    }
}

/// A sub-track of a multi-output instrument, routed into a bus: the two
/// fields whose absence silently corrupted a field analysis (ba doc
/// #273).
fn sub_track_summary() -> TrackSummary {
    TrackSummary {
        id: TrackId(5),
        name: "Drums → Kick".to_owned(),
        parent_id: Some(TrackId(4)),
        output: TrackOutput::Bus(TrackId(90)),
        ..track_summary()
    }
}

#[test]
fn track_summary_reports_parentage_and_routing() {
    let wire = serde_json::to_value(sub_track_summary()).unwrap();
    assert_eq!(wire["parent_id"], json!(4));
    // `"master"` or `{"bus_id": N}` — never a bare number.
    assert_eq!(wire["output"], json!({"bus_id": 90}));
    assert_eq!(wire["volume_db"], json!(-6.0));

    // An ordinary track omits `parent_id` entirely rather than emitting
    // `null` noise on every line.
    let wire = serde_json::to_value(track_summary()).unwrap();
    assert!(wire.get("parent_id").is_none(), "{wire}");
    assert_eq!(wire["output"], json!("master"));

    let back: TrackSummary = serde_json::from_value(
        serde_json::to_value(sub_track_summary()).unwrap(),
    )
    .unwrap();
    assert_eq!(back, sub_track_summary());
}

#[test]
fn song_summary_matches_documented_shape() {
    let summary = SongSummary {
        tempo_bpm: 120.0,
        time_signature: TimeSignature {
            numerator: 4,
            denominator: 4,
        },
        key: Some(KeyScale {
            tonic: "A".to_owned(),
            scale: "minor".to_owned(),
        }),
        sample_rate: 48000,
        length_bars: 16.0,
        length_samples: 1_536_000,
        transport: TransportState::Playing,
        playhead: SongPosition {
            bar: 3,
            beat: 1.5,
            sample: 192_000,
        },
        sections: vec![SectionPlacementView {
            id: SectionPlacementId(10),
            definition_id: SectionDefinitionId(1),
            name: "Verse".to_owned(),
            start_bar: 1,
            length_bars: 8,
        }],
        tracks: vec![track_summary()],
        revision: 12,
    };
    assert_eq!(
        serde_json::to_value(&summary).unwrap(),
        json!({
            "tempo_bpm": 120.0,
            "time_signature": {"numerator": 4, "denominator": 4},
            "key": {"tonic": "A", "scale": "minor"},
            "sample_rate": 48000,
            "length_bars": 16.0,
            "length_samples": 1_536_000,
            "transport": "playing",
            "playhead": {"bar": 3, "beat": 1.5, "sample": 192_000},
            "sections": [{
                "id": 10, "definition_id": 1, "name": "Verse",
                "start_bar": 1, "length_bars": 8
            }],
            "tracks": [{
                "id": 4, "name": "Bass", "kind": "instrument",
                "instrument": "resonance-wavetable",
                "muted": false, "soloed": true,
                "volume": 0.5, "volume_db": -6.0, "pan": -0.25,
                "output": "master", "clip_count": 2
            }],
            "revision": 12
        })
    );
    let back: SongSummary =
        serde_json::from_value(serde_json::to_value(&summary).unwrap()).unwrap();
    assert_eq!(back, summary);
}

#[test]
fn sections_view_roundtrips_with_chord_symbols() {
    let view = SectionsView {
        definitions: vec![SectionDefinitionView {
            id: SectionDefinitionId(1),
            name: "Chorus".to_owned(),
            length_bars: 8,
            scale: None,
            chords: vec![ChordView {
                id: ChordId(3),
                start_beat: 0.0,
                duration_beats: 4.0,
                symbol: "Am7".to_owned(),
            }],
        }],
        placements: vec![SectionPlacementView {
            id: SectionPlacementId(20),
            definition_id: SectionDefinitionId(1),
            name: "Chorus".to_owned(),
            start_bar: 9,
            length_bars: 8,
        }],
        revision: 4,
    };
    let wire = serde_json::to_value(&view).unwrap();
    assert_eq!(
        wire["definitions"][0]["chords"][0],
        json!({"id": 3, "start_beat": 0.0, "duration_beats": 4.0, "symbol": "Am7"})
    );
    // A definition without a scale omits the field entirely.
    assert!(wire["definitions"][0].get("scale").is_none());
    let back: SectionsView = serde_json::from_value(wire).unwrap();
    assert_eq!(back, view);
}

#[test]
fn track_detail_flattens_the_summary_fields() {
    let detail = TrackDetail {
        summary: track_summary(),
        effects: vec!["resonance-eq".to_owned()],
        frozen: false,
        clips: vec![ClipView {
            id: ClipId(7),
            name: Some("Bass groove".to_owned()),
            start: SongPosition {
                bar: 1,
                beat: 1.0,
                sample: 0,
            },
            length_beats: 16.0,
            length_samples: 384_000,
            midi: true,
        }],
    };
    let view = TracksView {
        tracks: vec![detail.clone()],
        revision: 2,
    };
    let wire = serde_json::to_value(&view).unwrap();
    // Summary fields sit at the top level of the track object.
    assert_eq!(wire["tracks"][0]["id"], json!(4));
    assert_eq!(wire["tracks"][0]["kind"], json!("instrument"));
    assert_eq!(wire["tracks"][0]["effects"], json!(["resonance-eq"]));
    assert_eq!(wire["tracks"][0]["clips"][0]["midi"], json!(true));
    let back: TracksView = serde_json::from_value(wire).unwrap();
    assert_eq!(back, view);
}

#[test]
fn notes_view_reports_both_pitch_and_positions() {
    let view = NotesView {
        clip_id: ClipId(7),
        notes: vec![NoteView {
            id: None,
            index: 0,
            pitch: 60,
            pitch_name: "C4".to_owned(),
            start_tick: 0,
            start_beat: 0.0,
            duration_ticks: 480,
            duration_beats: 1.0,
            velocity: 100,
        }],
        revision: 8,
    };
    let wire = serde_json::to_value(&view).unwrap();
    assert_eq!(
        wire["notes"][0],
        json!({
            "index": 0,
            "pitch": 60, "pitch_name": "C4",
            "start_tick": 0, "start_beat": 0.0,
            "duration_ticks": 480, "duration_beats": 1.0,
            "velocity": 100
        })
    );
    let back: NotesView = serde_json::from_value(wire).unwrap();
    assert_eq!(back, view);
}

#[test]
fn vocal_view_roundtrips_with_overrides_and_lowercase_state() {
    let mut overrides = BTreeMap::new();
    overrides.insert("lilia".to_owned(), vec!["l".to_owned(), "ih".to_owned()]);
    let view = VocalView {
        track_id: TrackId(9),
        lanes: vec![VocalLaneView {
            definition_id: SectionDefinitionId(3),
            name: "Verse".to_owned(),
            start_bar: Some(1),
            note_count: 4,
            syllable_count: 3,
            counts_mismatch: true,
            voicebank: Some("Lilia".to_owned()),
            comfortable_range: Some(PitchRangeView {
                low: 50,
                high: 79,
                low_name: "D3".to_owned(),
                high_name: "G5".to_owned(),
            }),
            notes: vec![VocalNoteView {
                index: 0,
                syllable: "hel".to_owned(),
                phonemes: vec!["hh".to_owned(), "eh".to_owned(), "l".to_owned()],
                phoneme_count: 3,
                pitch: 64,
                pitch_name: "E4".to_owned(),
                duration_ms: 214.0,
                min_duration_ms: 115.0,
                too_short: false,
                out_of_range: false,
            }],
            short_note_count: 0,
            out_of_range_note_count: 0,
        }],
        lines: vec![LyricLineView {
            index: 0,
            text: "hello world".to_owned(),
            syllables: vec![SyllableView {
                text: "hel".to_owned(),
                phonemes: vec!["hh".to_owned(), "eh".to_owned()],
            }],
        }],
        pronunciation_overrides: overrides,
        render_state: VocalRenderState::NotRendered,
        revision: 1,
    };
    let wire = serde_json::to_value(&view).unwrap();
    assert_eq!(wire["render_state"], json!("not_rendered"));
    assert_eq!(wire["pronunciation_overrides"]["lilia"], json!(["l", "ih"]));
    assert_eq!(wire["lanes"][0]["definition_id"], json!(3));
    assert_eq!(wire["lanes"][0]["counts_mismatch"], json!(true));
    let back: VocalView = serde_json::from_value(wire).unwrap();
    assert_eq!(back, view);

    // `lanes` is skipped when empty, and an older reply without it still
    // deserializes (the top-level fields kept their meaning).
    let mut legacy = view.clone();
    legacy.lanes.clear();
    let wire = serde_json::to_value(&legacy).unwrap();
    assert!(wire.get("lanes").is_none(), "empty lanes is omitted");
    let back: VocalView = serde_json::from_value(wire).unwrap();
    assert_eq!(back, legacy);
}

#[test]
fn track_kind_is_lowercase_and_forward_compatible() {
    for (kind, wire) in [
        (TrackKind::Instrument, "instrument"),
        (TrackKind::Drums, "drums"),
        (TrackKind::Vocal, "vocal"),
        (TrackKind::Audio, "audio"),
        (TrackKind::Bus, "bus"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire));
    }
    let future: TrackKind = serde_json::from_value(json!("hologram")).unwrap();
    assert_eq!(future, TrackKind::Unknown);
}

#[test]
fn views_tolerate_unknown_fields() {
    let back: NoteView = serde_json::from_value(json!({
        "index": 1,
        "pitch": 64, "pitch_name": "E4",
        "start_tick": 480, "start_beat": 1.0,
        "duration_ticks": 240, "duration_beats": 0.5,
        "velocity": 90,
        "field_from_the_future": [1, 2, 3]
    }))
    .unwrap();
    assert_eq!(back.pitch_name, "E4");
    assert_eq!(back.id, None);
}
