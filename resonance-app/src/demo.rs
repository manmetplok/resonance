//! Demo-content fixture. Originally invoked by a `--demo` CLI flag for
//! screenshot capture; now exposed as a public test fixture so that
//! integration tests (`iced_test`-driven snapshots) can populate the
//! GUI-side state with tracks, busses, clips, and a Compose section
//! without booting the audio engine.
//!
//! The runtime never calls this — it lives in the library crate purely
//! so `resonance-app/tests/*.rs` can call it via `resonance_app::demo`.

use resonance_audio::types::{FadeCurve, MidiNote, TrackId, TrackOutput};

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

    // Bar length straight from the tempo map rebuilt above, so clip
    // placement can never drift from the ruler again. This used to be
    // `60/bpm * 6 * sr`, i.e. a 6/4 bar, while the clips' notes below
    // were written against a 6/8 one (ba todo #1389).
    let bar_samples = app.tempo_map.bar_to_sample(1);
    app.master.level_l = 0.62;
    app.master.level_r = 0.48;

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
    app.interaction.select_single_track(Some(2));
    // Demo seed bypasses the engine-event handlers that normally keep
    // this cache fresh, so refresh by hand.
    app.compose.refresh_track_count(&app.registry.tracks);

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
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks,
        vocal_tuning: None,
        asset_ref: None,
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
        arrangement: Vec::new(),
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

    // Seeding mutated transport/scale state directly (not via
    // `update()`), so re-derive the transport label cache by hand —
    // snapshot tests render `view()` immediately after seeding.
    app.refresh_transport_labels();
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

/// Seed for the **Mixer sub-track grouping** regression. Builds a
/// project where a parent `Drums` track has four sub-tracks but the
/// `.order` field on those sub-tracks deliberately lands them *after*
/// several unrelated tracks — the same shape the engine produces in
/// practice, because `next_track_order` keeps climbing as the user
/// works. Used by `tests/mixer_sub_track_grouping.rs` to verify the
/// mixer renders the sub-strips grouped with their parent regardless
/// of `.order`, and to lock in the visual treatment (recessed strip
/// background + lavender left-rail).
pub fn seed_demo_with_drum_subtracks(app: &mut Resonance) {
    use resonance_audio::types::TrackOutput;

    app.io.has_active_project = true;
    app.transport.bpm = 120.0;
    app.transport.bpm_input = "120.0".to_string();
    app.transport.time_sig_num = 4;
    app.transport.time_sig_den = 4;
    app.tempo_events = vec![state::TempoEvent { bar: 0, bpm: 120.0 }];
    app.signature_events = vec![state::SignatureEvent {
        bar: 0,
        numerator: 4,
        denominator: 4,
    }];
    app.rebuild_tempo_map();

    // Parent drum track at order 0. The plugin slot gives it the
    // instrument pill the strip renders.
    let mut drums =
        TrackState::new_instrument(1, 0);
    drums.name = "Drums".to_string();
    drums.instrument_type = state::InstrumentType::Drum;
    drums.instrument_icon = state::InstrumentIcon::Drum;
    drums.plugins.push(PluginSlotState::new(
        100,
        "Resonance Drums".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    drums.level_l = 0.55;
    drums.level_r = 0.50;
    drums.output = TrackOutput::Master;

    // Three unrelated tracks at orders 1..=3. With the old linear
    // render walk these would render between the parent strip and its
    // sub-strips; the new grouping pass must keep them after the
    // entire `Drums + sub-tracks` cluster.
    let mut bass = TrackState::new_instrument(2, 1);
    bass.name = "Synth Bass".to_string();
    bass.instrument_icon = state::InstrumentIcon::Music;
    bass.plugins.push(PluginSlotState::new(
        200,
        "Resonance Wave".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    bass.level_l = 0.48;
    bass.level_r = 0.42;

    let mut pad = TrackState::new_instrument(3, 2);
    pad.name = "Synth Pad".to_string();
    pad.instrument_icon = state::InstrumentIcon::WaveSquare;
    pad.plugins.push(PluginSlotState::new(
        300,
        "Resonance Wave".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    pad.level_l = 0.30;
    pad.level_r = 0.28;

    let mut lead = TrackState::new_instrument(4, 3);
    lead.name = "Lead".to_string();
    lead.instrument_icon = state::InstrumentIcon::Music;
    lead.plugins.push(PluginSlotState::new(
        400,
        "Resonance Wave".to_string(),
        String::new(),
        String::new(),
        Vec::new(),
        false,
    ));
    lead.level_l = 0.40;
    lead.level_r = 0.36;

    // Four sub-tracks of `Drums` allocated *after* the unrelated
    // tracks — orders 4..=7. This is exactly the layout the engine
    // produces in practice (sub-tracks are pushed onto the end of the
    // registry when the drum plugin reports its output ports).
    let mut kick = TrackState::new_sub_track(10, 4, "Drums \u{2192} Kick".to_string(), 1, 1);
    kick.level_l = 0.62;
    kick.level_r = 0.58;
    let mut snare = TrackState::new_sub_track(11, 5, "Drums \u{2192} Snare".to_string(), 1, 2);
    snare.level_l = 0.50;
    snare.level_r = 0.46;
    let mut hh = TrackState::new_sub_track(12, 6, "Drums \u{2192} HH".to_string(), 1, 3);
    hh.level_l = 0.35;
    hh.level_r = 0.32;
    let mut tom = TrackState::new_sub_track(13, 7, "Drums \u{2192} Tom".to_string(), 1, 4);
    tom.level_l = 0.28;
    tom.level_r = 0.26;

    app.registry.tracks = vec![drums, bass, pad, lead, kick, snare, hh, tom];
    app.registry.next_track_order = 8;
    app.registry.next_sub_track_id = 14;
    app.interaction.select_single_track(Some(1));

    // Expand the drum parent so the mixer renders its sub-strips —
    // the whole point of the visual is verified in that state.
    app.mixer.expanded_sub_track_parents.insert(1);

    // Demo seed bypasses engine-event handlers that normally refresh
    // these caches, so do it by hand.
    app.view_caches.rebuild_output(&app.registry.busses);
    app.compose.refresh_track_count(&app.registry.tracks);
    app.refresh_transport_labels();
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
    app.interaction.select_single_track(Some(1));
    app.compose.refresh_track_count(&app.registry.tracks);
    app.refresh_transport_labels();

    // Intentionally no busses and no `view_caches.rebuild_output` —
    // this is the state that used to panic when the Mixer tab opened.
}

/// Seed `n` synth-instrument tracks named `"VirtTrack 1..=n"` for the
/// track-header column virtualization tests. Each track gets its own
/// id and order so `sorted_tracks` ordering matches the seed order.
/// Bypasses the audio engine — the virtualization logic under test
/// reads `r.registry.tracks` and `r.viewport` only.
pub fn seed_many_synth_tracks(app: &mut Resonance, n: usize) {
    app.io.has_active_project = true;
    app.registry.tracks.clear();
    for i in 0..n {
        let mut t = TrackState::new_instrument(1_000 + i as u64, i);
        t.name = format!("VirtTrack {}", i + 1);
        app.registry.tracks.push(t);
    }
    app.registry.next_track_order = n;
    app.compose.refresh_track_count(&app.registry.tracks);
    app.refresh_transport_labels();
}

/// Seed three pool assets into the browser so the Pool tab renders with
/// content for snapshot tests (todo #603):
///
/// * **Asset 1 — used**: a stereo WAV, 4.5 s, referenced by the first
///   audio clip already present in the registry (if any — call after
///   `seed_demo_content`).
/// * **Asset 2 — unused**: a mono FLAC, 2.0 s, not referenced by any clip.
/// * **Asset 3 — missing**: a WAV flagged `missing`, representing a file
///   the project can no longer locate.
///
/// Calls `recompute_pool_usage` so usage badges render correctly.
pub fn seed_pool_assets(app: &mut Resonance) {
    use resonance_common::AudioFormat;

    // Build a gentle sinusoidal waveform for thumbnail peaks.
    let make_peaks = |count: usize, phase: f32| -> Vec<(f32, f32)> {
        (0..count)
            .map(|i| {
                let t = i as f32 / count as f32;
                let amp = 0.3 + 0.5 * (t * std::f32::consts::TAU + phase).sin().abs();
                (-amp, amp)
            })
            .collect()
    };

    let asset1 = crate::state::PoolAsset {
        id: 1,
        project_relative_path: "audio/asset_1.wav".to_string(),
        original_path: "/sessions/My Project/audio/Kick Loop 120bpm.wav".to_string(),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 44_100,
        duration_frames: (44_100.0 * 4.5) as u64,
        thumbnail_peaks: make_peaks(48, 0.0),
        missing: false,
    };

    let asset2 = crate::state::PoolAsset {
        id: 2,
        project_relative_path: "audio/asset_2.wav".to_string(),
        original_path: "/sessions/My Project/audio/Clap One-Shot.flac".to_string(),
        format: AudioFormat::Flac,
        channels: 1,
        source_sample_rate: 48_000,
        duration_frames: (44_100.0 * 2.0) as u64,
        thumbnail_peaks: make_peaks(48, 1.2),
        missing: false,
    };

    let asset3 = crate::state::PoolAsset {
        id: 3,
        project_relative_path: "audio/asset_3.wav".to_string(),
        original_path: "/sessions/My Project/audio/Riser FX.wav".to_string(),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 44_100,
        duration_frames: (44_100.0 * 8.0) as u64,
        thumbnail_peaks: make_peaks(48, 2.5),
        missing: true,
    };

    app.media.pool.add(asset1);
    app.media.pool.add(asset2);
    app.media.pool.add(asset3);

    // Link the first audio clip to asset 1 so it renders as "used ×1".
    if let Some(clip) = app.clips.first_mut() {
        clip.asset_ref = Some(crate::state::AssetRef::new(1));
    }

    app.recompute_pool_usage();
}

/// Seed the Files tab of the media browser with a populated folder so it
/// renders with content for snapshot tests (todo #602): a current folder
/// (favourited, so the breadcrumb star reads WARM), a favourites / recent
/// shelf, two subfolders, and four audio rows spanning the format chips
/// (wav / flac / mp3 / ogg) with decoded-style waveform thumbnails.
pub fn seed_files_folder(app: &mut Resonance) {
    use resonance_common::audio_probe::{AudioFileEntry, AudioFormat, AudioInfo};
    use std::path::PathBuf;

    let root = "/sessions/My Project/samples/Drums";
    let current = PathBuf::from(root);

    // A gentle sinusoidal silhouette so the thumbnails read as waveforms.
    let make_peaks = |count: usize, phase: f32| -> Vec<(f32, f32)> {
        (0..count)
            .map(|i| {
                let t = i as f32 / count as f32;
                let amp = 0.25 + 0.55 * (t * std::f32::consts::TAU + phase).sin().abs();
                (-amp, amp)
            })
            .collect()
    };

    let entry = |name: &str, format: AudioFormat, channels: u16, sr: u32, secs: f64| {
        let path = format!("{root}/{name}");
        AudioFileEntry {
            path,
            info: AudioInfo {
                format,
                channels,
                sample_rate: sr,
                frames: (sr as f64 * secs) as u64,
                duration_secs: secs,
            },
        }
    };

    let files = vec![
        entry("Kick 120bpm.wav", AudioFormat::Wav, 2, 44_100, 1.2),
        entry("Clap One-Shot.flac", AudioFormat::Flac, 1, 48_000, 0.4),
        entry("Groove Loop.mp3", AudioFormat::Mp3, 2, 44_100, 4.0),
        entry("Ambient Pad.ogg", AudioFormat::Ogg, 2, 44_100, 8.5),
    ];

    let mut thumbnails = std::collections::HashMap::new();
    for (i, f) in files.iter().enumerate() {
        thumbnails.insert(f.path.clone(), make_peaks(48, i as f32 * 0.9));
    }

    app.media.browser.current_folder = Some(current.clone());
    app.media.browser.scanning = false;
    app.media.browser.filter.clear();
    app.media.browser.scan = crate::state::FolderScan {
        folders: vec![
            PathBuf::from(format!("{root}/Kicks")),
            PathBuf::from(format!("{root}/Snares")),
        ],
        files,
        thumbnails,
    };

    // Favourites (WARM star) + recent (clock) shelf. The current folder is a
    // favourite so the breadcrumb star reads pinned.
    app.media.pool.favourites = vec![PathBuf::from("/Users/me/Loops"), current.clone()];
    app.media.pool.recent_folders = vec![current, PathBuf::from("/Users/me/Vocals")];
}

/// Seed the Files tab on an **empty** folder — one with no audio — so the
/// empty-folder state renders for snapshot tests (todo #602).
pub fn seed_empty_files_folder(app: &mut Resonance) {
    use std::path::PathBuf;

    app.media.browser.current_folder = Some(PathBuf::from("/sessions/My Project/samples/Empty"));
    app.media.browser.scanning = false;
    app.media.browser.filter.clear();
    app.media.browser.scan = crate::state::FolderScan::default();

    app.media.pool.favourites = vec![PathBuf::from("/Users/me/Loops")];
    app.media.pool.recent_folders = vec![PathBuf::from("/Users/me/Vocals")];
}

/// Seed the audition transport in its **idle** state for snapshots (todo
/// #604): the Files folder is open with a row selected-to-audition but no
/// preview sounding and the three toggles off. The transport shows the play
/// button, the selected row's waveform, a `0:00 / M:SS` readout, and the
/// neutral toggle chips.
pub fn seed_audition_idle(app: &mut Resonance) {
    use std::path::PathBuf;

    seed_files_folder(app);
    let selected = PathBuf::from(app.media.browser.scan.files[0].path.clone());
    app.media.browser.audition.selected = Some(selected);
    app.media.browser.audition.playing = None;
    app.media.browser.audition.position_frame = 0;
    app.media.browser.audition.auto_play = false;
    app.media.browser.audition.loop_enabled = false;
    app.media.browser.audition.sync_to_tempo = false;
}

/// Seed the audition transport **playing** a row for snapshots (todo #604):
/// a preview sounding ~40 % through with Auto-play + Loop + Sync-to-tempo on,
/// so the playing row reads WARM, the scrub playhead sits mid-strip, the
/// played span colours WARM, and the readout advances.
pub fn seed_audition_playing(app: &mut Resonance) {
    use std::path::PathBuf;

    seed_files_folder(app);
    // "Groove Loop.mp3" (index 2) is a 4 s row — long enough that a 40 %
    // playhead reads clearly mid-strip.
    let entry = app.media.browser.scan.files[2].clone();
    let path = PathBuf::from(&entry.path);
    app.media.browser.audition.selected = Some(path.clone());
    app.media.browser.audition.playing = Some(path);
    app.media.browser.audition.position_frame = (entry.info.frames as f64 * 0.4) as u64;
    app.media.browser.audition.auto_play = true;
    app.media.browser.audition.loop_enabled = true;
    app.media.browser.audition.sync_to_tempo = true;
}
