//! Tests for the built-in starter project skeletons (todo #664).
//! Separate file as required — no inline #[cfg(test)] modules.
//!
//! Each builder must yield a valid `ProjectFile` that replays into the
//! routing the design describes, with summary chips that match the actual
//! contents.

use resonance_app::compose::LaneGeneratorKind;
use resonance_app::project::ProjectFile;
use resonance_app::state::InstrumentType;
use resonance_app::update::project_io::{
    builtin_templates, compute_summary, BuiltinTemplateId,
};
use resonance_music_theory::VocalVoicebank;

/// Every builder's `ProjectFile` must survive a JSON round-trip — the
/// strongest proxy for "valid ProjectFile" without booting the engine,
/// since instantiation serializes/deserializes through the same path.
#[test]
fn all_builtins_serialize_round_trip() {
    for id in BuiltinTemplateId::ALL {
        let built = id.build();
        let json = serde_json::to_string_pretty(&built.file)
            .unwrap_or_else(|e| panic!("{}: serialize failed: {e}", id.slug()));
        let back: ProjectFile = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("{}: deserialize failed: {e}", id.slug()));
        assert_eq!(
            back.version, built.file.version,
            "{}: version drifted on round-trip",
            id.slug()
        );
        // Built-ins are always captured at the current format version.
        assert_eq!(built.file.version, resonance_app::project::PROJECT_FORMAT_VERSION);
    }
}

/// The picker exposes exactly the four starters, in order, each with a
/// non-empty name + description and a summary computed from real contents.
#[test]
fn builtin_templates_list_matches_ids() {
    let templates = builtin_templates();
    assert_eq!(templates.len(), 4);
    for (tpl, id) in templates.iter().zip(BuiltinTemplateId::ALL) {
        assert_eq!(tpl.name, id.display_name());
        assert_eq!(tpl.description, id.description());
        assert!(!tpl.name.is_empty() && !tpl.description.is_empty());
        // Built-in Templates carry the builtin kind and the current schema.
        assert!(matches!(
            tpl.kind,
            resonance_app::update::project_io::TemplateKind::Builtin
        ));
    }
}

/// Slugs are stable and round-trip through `from_slug`.
#[test]
fn slug_round_trip() {
    for id in BuiltinTemplateId::ALL {
        assert_eq!(BuiltinTemplateId::from_slug(id.slug()), Some(id));
    }
    assert_eq!(BuiltinTemplateId::from_slug("nope"), None);
    // Slugs are unique.
    let mut slugs: Vec<&str> = BuiltinTemplateId::ALL.iter().map(|id| id.slug()).collect();
    slugs.sort_unstable();
    slugs.dedup();
    assert_eq!(slugs.len(), 4);
}

/// For every builder, the descriptor's summary chips must equal a summary
/// recomputed from the project — they can never drift.
#[test]
fn summaries_match_contents() {
    for id in BuiltinTemplateId::ALL {
        let built = id.build();
        let from_project = compute_summary(&built.file);
        let from_descriptor = id.template().summary;

        assert_eq!(from_descriptor.track_count, from_project.track_count, "{}", id.slug());
        assert_eq!(from_descriptor.bus_count, from_project.bus_count, "{}", id.slug());
        assert_eq!(from_descriptor.plugin_count, from_project.plugin_count, "{}", id.slug());
        assert_eq!(from_descriptor.tempo_bpm, from_project.tempo_bpm, "{}", id.slug());
        assert_eq!(from_descriptor.time_sig, from_project.time_sig, "{}", id.slug());

        // The summary counts must reflect the actual collections.
        assert_eq!(from_project.track_count, built.file.tracks.len(), "{}", id.slug());
        assert_eq!(from_project.bus_count, built.file.busses.len(), "{}", id.slug());
        let plugin_total: usize = built.file.tracks.iter().map(|t| t.plugins.len()).sum::<usize>()
            + built.file.busses.iter().map(|b| b.plugins.len()).sum::<usize>()
            + built.file.master_plugins.len();
        assert_eq!(from_project.plugin_count, plugin_total, "{}", id.slug());
    }
}

#[test]
fn empty_is_blank_defaults() {
    let built = BuiltinTemplateId::Empty.build();
    assert!(built.file.tracks.is_empty());
    assert!(built.file.busses.is_empty());
    assert!(built.file.master_plugins.is_empty());
    assert!(built.file.midi_clips.is_empty());
    assert!(built.file.section_definitions.is_empty());
    assert!(built.midi_notes.is_empty());
    // Default tempo / time-sig.
    assert_eq!(built.file.bpm, 120.0);
    assert_eq!(built.file.time_sig_num, 4);
    assert_eq!(built.file.time_sig_den, 4);
}

#[test]
fn band_recording_routing() {
    let built = BuiltinTemplateId::BandRecording.build();
    let f = &built.file;

    // Six audio tracks.
    assert_eq!(f.tracks.len(), 6);
    assert!(f.tracks.iter().all(|t| t.track_type == "audio"));

    // Two busses, found by name.
    assert_eq!(f.busses.len(), 2);
    let drum_bus = f.busses.iter().find(|b| b.name == "Drums").expect("drum bus");
    let inst_bus = f.busses.iter().find(|b| b.name == "Instruments").expect("inst bus");

    // Every track routes to one of the two busses; the split is 3/3.
    assert!(f.tracks.iter().all(|t| t.output_bus.is_some()));
    let to_drums = f.tracks.iter().filter(|t| t.output_bus == Some(drum_bus.id)).count();
    let to_inst = f.tracks.iter().filter(|t| t.output_bus == Some(inst_bus.id)).count();
    assert_eq!(to_drums, 3);
    assert_eq!(to_inst, 3);

    // Master chain: EQ → Compressor → Mastering.
    let chain: Vec<&str> = f.master_plugins.iter().map(|p| p.clap_plugin_id.as_str()).collect();
    assert_eq!(
        chain,
        vec![
            "com.resonance.eq",
            "com.resonance.compressor",
            "com.resonance.mastering",
        ]
    );

    // No MIDI for a tracking session.
    assert!(built.midi_notes.is_empty());
}

#[test]
fn beatmaking_drum_synth_and_fx_returns() {
    let built = BuiltinTemplateId::Beatmaking.build();
    let f = &built.file;

    // A drum sampler and a wavetable synth, both instrument tracks.
    assert_eq!(f.tracks.len(), 2);
    let drums = &f.tracks[0];
    assert_eq!(drums.track_type, "instrument");
    assert_eq!(drums.instrument_type, InstrumentType::Drum);
    assert_eq!(drums.plugins[0].clap_plugin_id, "com.resonance.drums");

    let synth = &f.tracks[1];
    assert_eq!(synth.track_type, "instrument");
    assert_eq!(synth.plugins[0].clap_plugin_id, "com.resonance.wavetable");

    // Reverb + delay FX return busses.
    assert_eq!(f.busses.len(), 2);
    let reverb = f.busses.iter().find(|b| b.name == "Reverb").expect("reverb bus");
    let delay = f.busses.iter().find(|b| b.name == "Delay").expect("delay bus");
    assert_eq!(reverb.plugins[0].clap_plugin_id, "com.resonance.reverb");
    assert_eq!(delay.plugins[0].clap_plugin_id, "com.resonance.delay");
}

#[test]
fn vocal_songwriting_contents() {
    let built = BuiltinTemplateId::VocalSongwriting.build();
    let f = &built.file;

    // Lead vocal + pad/bass bed.
    assert_eq!(f.tracks.len(), 3);
    let vocal = f.tracks.iter().find(|t| t.track_type == "vocal").expect("vocal track");
    assert_eq!(vocal.name, "Lead Vocal");
    let inst_count = f.tracks.iter().filter(|t| t.track_type == "instrument").count();
    assert_eq!(inst_count, 2);
    assert!(f
        .tracks
        .iter()
        .filter(|t| t.track_type == "instrument")
        .all(|t| t.plugins[0].clap_plugin_id == "com.resonance.wavetable"));

    // Chord track: a section with a four-chord progression and a placement.
    assert_eq!(f.section_definitions.len(), 1);
    let section = &f.section_definitions[0];
    assert_eq!(section.chords.len(), 4);
    assert_eq!(f.section_placements.len(), 1);
    assert_eq!(f.section_placements[0].definition_id, section.id);

    // A Compose vocal line on the Lilia voicebank, keyed to the vocal track.
    let cfg = section.lane_generators.get(&vocal.id).expect("vocal lane generator");
    match &cfg.kind {
        LaneGeneratorKind::Vocal(params) => {
            assert_eq!(params.voicebank, VocalVoicebank::Lilia);
        }
        other => panic!("expected a Vocal lane generator, got {other:?}"),
    }

    // The melody is pre-baked: one MIDI clip on the vocal track, with notes
    // carried in `midi_notes` keyed by the clip id (no .mid file on disk).
    assert_eq!(f.midi_clips.len(), 1);
    let clip = &f.midi_clips[0];
    assert_eq!(clip.track_id, vocal.id);
    let notes = built.midi_notes.get(&clip.id).expect("baked vocal notes");
    assert!(!notes.is_empty(), "vocal melody should generate notes");
    // Notes stay within the clip's tick span.
    assert!(notes.iter().all(|n| n.start_tick < clip.duration_ticks));
}
