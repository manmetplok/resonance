//! Demo-content fixture. Originally invoked by a `--demo` CLI flag for
//! screenshot capture; now exposed as a public test fixture so that
//! integration tests (`iced_test`-driven snapshots) can populate the
//! GUI-side state with tracks, busses, clips, and a Compose section
//! without booting the audio engine.
//!
//! The runtime never calls this — it lives in the library crate purely
//! so `resonance-app/tests/*.rs` can call it via `resonance_app::demo`.

use resonance_audio::types::{MidiNote, TrackId, TrackOutput};

use crate::state::{self, BusState, ClipState, MidiClipState, PluginSlotState, TrackState};
use crate::Resonance;

/// Populate the GUI-side state with a small set of tracks, busses, clips,
/// and a Compose section so the views render with content for snapshots.
/// Bypasses the audio engine entirely — these objects exist only in the
/// app's `registry` / `compose` / `clips` collections and won't make sound.
pub fn seed_demo_content(app: &mut Resonance) {
    use resonance_audio::types::{MidiNote, SamplePos};
    use resonance_music_theory::{Chord, ChordQuality, Mode, MotifSource, PitchClass, Scale};

    use crate::compose::{
        ChordState, GenerateParams, SectionDefinitionState, SectionPlacementState,
    };

    app.io.has_active_project = true;
    app.transport.bpm = 90.0;
    app.transport.bpm_input = "90.0".to_string();
    app.transport.time_sig_num = 6;
    app.transport.time_sig_den = 8;
    app.tempo_events = vec![state::TempoEvent { bar: 0, bpm: 90.0 }];
    app.signature_events = vec![state::SignatureEvent {
        bar: 0,
        numerator: 6,
        denominator: 8,
    }];
    app.rebuild_tempo_map();

    let sr = app.sample_rate as u64;
    let secs_per_beat = 60.0 / app.transport.bpm as f64;
    let bar_samples = (secs_per_beat * 6.0 * sr as f64) as u64;
    app.master_level_l = 0.62;
    app.master_level_r = 0.48;

    // ---- Tracks ----
    let mk_instr = |id: u64,
                    order: usize,
                    name: &str,
                    plugin_name: &str,
                    icon: state::InstrumentIcon|
     -> TrackState {
        let mut t = TrackState::new_instrument(id, order);
        t.name = name.to_string();
        t.instrument_icon = icon;
        t.level_l = 0.5;
        t.level_r = 0.4;
        if !plugin_name.is_empty() {
            t.plugins.push(PluginSlotState::new(
                id * 100,
                plugin_name.to_string(),
                String::new(),
                String::new(),
                Vec::new(),
                false,
            ));
        }
        t
    };

    let mut drums = mk_instr(
        1,
        0,
        "Drums",
        "Resonance Drums",
        state::InstrumentIcon::Drum,
    );
    drums.instrument_type = state::InstrumentType::Drum;

    let bass = mk_instr(
        2,
        1,
        "Synth Bass",
        "Resonance Wave",
        state::InstrumentIcon::Music,
    );
    let pad = mk_instr(
        3,
        2,
        "Synth Pad",
        "Resonance Wave",
        state::InstrumentIcon::WaveSquare,
    );
    let lead = mk_instr(
        4,
        3,
        "Lead Synth",
        "Resonance Wave",
        state::InstrumentIcon::Music,
    );

    let mut audio = TrackState::new_audio(5, 4);
    audio.name = "Drums Bounce".to_string();
    audio.muted = true;
    audio.instrument_icon = state::InstrumentIcon::Microphone;

    // Lead Vocal is a `TrackType::Vocal` track — first-class engine flavour
    // that pairs a MIDI staff with a rendered SVS waveform. No instrument
    // plugin: the audio comes from the rendered WAV, not from a synth.
    const VOCAL_TRACK_ID: u64 = 6;
    let mut vocal = TrackState::new_vocal(VOCAL_TRACK_ID, 5);
    vocal.name = "Lead Vocal".to_string();
    vocal.level_l = 0.5;
    vocal.level_r = 0.4;

    app.registry.tracks = vec![drums, bass, pad, lead, audio, vocal];
    app.registry.next_track_order = 6;
    app.interaction.selected_track = Some(2);

    // ---- Busses ----
    app.registry.busses = vec![
        BusState::new(100, 0, "Bus 1 · Drums".to_string()),
        BusState::new(101, 1, "Bus 2 · FX".to_string()),
    ];
    app.registry.busses[0].plugins.push(PluginSlotState::new(
        10001,
        "Comp".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    app.registry.busses[0].level_l = 0.55;
    app.registry.busses[0].level_r = 0.50;
    app.registry.busses[1].plugins.push(PluginSlotState::new(
        10002,
        "Verb".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    app.registry.busses[1].level_l = 0.32;
    app.registry.busses[1].level_r = 0.30;
    app.registry.next_bus_order = 2;
    // Demo seed bypasses the engine event handlers that normally
    // refresh these caches, so refresh them by hand.
    app.view_caches.rebuild_output(&app.registry.busses);

    // ---- Clips on the timeline ----
    let bar_ticks = 480 * 6 / 2; // 6/8 → 6 eighth-note beats per bar
    let make_midi_clip = |id: u64,
                          track: u64,
                          name: &str,
                          start_bar: u64,
                          length_bars: u64,
                          density: u32|
     -> MidiClipState {
        let mut notes = Vec::new();
        let total_ticks = length_bars * bar_ticks;
        let step = (total_ticks / density as u64).max(60);
        let mut tick = 0u64;
        let mut pitch = 60u8;
        let mut i = 0u32;
        while tick < total_ticks {
            notes.push(MidiNote {
                note: pitch,
                velocity: 0.8,
                start_tick: tick,
                duration_ticks: (step * 9) / 10,
            });
            tick += step;
            i += 1;
            pitch = 48 + ((i * 5) % 24) as u8;
        }
        MidiClipState {
            id,
            track_id: track,
            start_sample: (start_bar * bar_samples) as SamplePos,
            duration_ticks: total_ticks,
            name: name.to_string(),
            notes,
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        }
    };

    app.midi_clips = vec![
        make_midi_clip(11, 1, "Pattern A", 0, 6, 32),
        make_midi_clip(12, 2, "Bm progression", 0, 6, 12),
        make_midi_clip(13, 3, "Pad", 0, 6, 8),
        make_midi_clip(14, 4, "Motif", 0, 6, 20),
        // Vocal melody clip is appended after the section is constructed
        // so the chord progression is available to `derive_vocal`. The
        // placeholder entry is replaced below.
    ];

    // Audio bounce on track 5 — uses peaks rather than a real waveform.
    let peak_count = 256usize;
    let waveform_peaks = (0..peak_count)
        .map(|i| {
            let t = i as f32 / peak_count as f32;
            let amp = 0.4 + 0.4 * (t * 12.0).sin().abs();
            (-amp, amp)
        })
        .collect();
    app.clips = vec![ClipState {
        id: 15,
        track_id: 5,
        start_sample: 0,
        duration_samples: bar_samples * 5 + bar_samples / 2,
        name: "Drums bounce".to_string(),
        total_frames: bar_samples * 5 + bar_samples / 2,
        trim_start_frames: 0,
        trim_end_frames: 0,
        waveform_peaks,
    }];

    // Place the playhead a bit into the song so it's visible.
    app.transport.playhead = bar_samples * 4;

    // ---- Compose section ----
    let def_id = app.compose.fresh_id();
    let chords = [
        Chord::new(PitchClass::B, ChordQuality::Min),
        Chord::new(PitchClass::B, ChordQuality::Min),
        Chord::new(PitchClass::Fs, ChordQuality::Maj),
        Chord::new(PitchClass::G, ChordQuality::Maj),
        Chord::new(PitchClass::E, ChordQuality::Min),
    ];
    let chord_states: Vec<ChordState> = chords
        .iter()
        .enumerate()
        .map(|(i, c)| ChordState {
            id: app.compose.fresh_id(),
            start_beat: i as u32 * 4,
            duration_beats: 4,
            chord: *c,
        })
        .collect();

    // Pre-seed the Vocal lane generator on the Lead Vocal track so the
    // demo lands on the new design without the user having to flip the
    // generator picker.
    let mut lane_generators = std::collections::HashMap::new();
    lane_generators.insert(
        VOCAL_TRACK_ID,
        crate::compose::LaneGeneratorConfig {
            kind: crate::compose::LaneGeneratorKind::Vocal(
                resonance_music_theory::VocalParams::default(),
            ),
            seed: 0x00C0_FFEE_FACE_F00D,
        },
    );

    app.compose.definitions.push(SectionDefinitionState {
        id: def_id,
        name: "Intro".to_string(),
        color: [139, 109, 255],
        length_bars: 8,
        chords: chord_states,
        scale: Some(Scale::new(PitchClass::B, Mode::Minor)),
        progression_seed: 12345,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators,
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
    });

    let placement_id = app.compose.fresh_id();
    app.compose.placements.push(SectionPlacementState {
        id: placement_id,
        definition_id: def_id,
        start_bar: 0,
    });
    app.compose.selected_placement_id = Some(placement_id);

    // Land in the Drums lane so the new drum-groups design surfaces are
    // visible on first boot. Switching back to Vocal is a single click in
    // the right-rail lane switcher.
    // Land in the Lead Vocal lane so the new design surfaces are visible.
    app.compose.selected_lane = crate::compose::SelectedLane::Instrument(VOCAL_TRACK_ID);
    crate::update::compose::ensure_vocal_bulk_lyrics_for_selection(app);

    // Pre-generate the vocal melody so the staff shows real notes on
    // first boot instead of the synthetic contour fallback.
    seed_demo_vocal_melody(app, def_id, placement_id, VOCAL_TRACK_ID, 16);

    // Materialise the project's drum groups into a MIDI clip on the
    // drum track so the demo plays back something audible on the kit
    // without an explicit Generate press.
    crate::update::compose::drum_groups::materialize_drum_clips(app);

    let _ = TrackOutput::Master; // silence unused-import warning when feature flags shift

    // Demo seed bypasses the engine event handlers that maintain
    // `plugin_index`; rebuild it once from the freshly-seeded state so
    // `with_plugin_mut` can locate demo plugins without falling back to
    // a linear scan.
    app.rebuild_plugin_index();
}

/// Pre-bake a vocal MIDI clip for the demo content. Walks the section's
/// chord progression through `derive_vocal`, materialises a `MidiClipState`
/// at `clip_id`, and registers it in `compose.derived_clips` so the
/// vocal lane finds it on first paint.
fn seed_demo_vocal_melody(
    app: &mut Resonance,
    def_id: u64,
    placement_id: u64,
    track_id: TrackId,
    clip_id: u64,
) {
    use resonance_audio::types::TICKS_PER_QUARTER_NOTE as TPQN;
    let Some(def) = app.compose.find_definition(def_id).cloned() else {
        return;
    };
    let Some(cfg) = def.lane_generators.get(&track_id).cloned() else {
        return;
    };
    let crate::compose::LaneGeneratorKind::Vocal(params) = cfg.kind else {
        return;
    };
    let timed = crate::compose::generate::to_timed_chords(&def.chords);
    let notes = resonance_music_theory::derive_vocal(&timed, &params, TPQN as u32, cfg.seed);
    let duration_ticks = def.length_bars as u64 * app.transport.time_sig_num as u64 * TPQN;
    let midi_notes: Vec<MidiNote> = notes
        .iter()
        .map(|n| MidiNote {
            note: n.note,
            velocity: n.velocity,
            start_tick: n.start_tick,
            duration_ticks: n.duration_ticks,
        })
        .collect();
    app.midi_clips.push(MidiClipState {
        id: clip_id,
        track_id,
        start_sample: 0,
        duration_ticks,
        name: format!("{} \u{00B7} Lead Vocal", def.name),
        notes: midi_notes,
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app.compose
        .derived_clips
        .insert((def_id, placement_id, track_id), clip_id);
}

/// Minimal seed for the "fresh-project + one track + open Mixer"
/// regression. Mirrors what the app looks like the instant after the
/// user adds their first track (e.g. the preset Drums track) to a
/// brand-new empty project: a single instrument track in the registry,
/// it selected in the inspector, no busses, and crucially *no*
/// `view_caches.rebuild_output` call — so `output_choices` stays at
/// the default the constructor produced. Used by
/// `tests/mixer_inspector_empty_project.rs` to lock in the fix for
/// the panic at `view/mixer/inspector.rs:450`
/// (`index out of bounds: the len is 0 but the index is 0`).
pub fn seed_minimal_drum_track_no_busses(app: &mut Resonance) {
    app.io.has_active_project = true;

    let mut drums = TrackState::new_instrument(1, 0);
    drums.name = "Drums".to_string();
    drums.instrument_type = state::InstrumentType::Drum;
    drums.instrument_icon = state::InstrumentIcon::Drum;
    drums.output = TrackOutput::Master;

    app.registry.tracks = vec![drums];
    app.registry.next_track_order = 1;
    app.interaction.selected_track = Some(1);

    // Intentionally no busses and no `view_caches.rebuild_output` —
    // this is the state that used to panic when the Mixer tab opened.
}
