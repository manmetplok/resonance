//! The whole audio callback against render-graph edits published
//! concurrently (code review ARCH-02 A2-4/A2-5, refactor todos B-1/B-2).
//!
//! Before A2-4 the playing branch `try_read` the MIDI-clip, bus and
//! master locks, and an edit on the engine thread (or a writer queued
//! behind a worker's read guard) made it render silence for the block —
//! or, for the master lock, drop the master FX pass. Those maps now come
//! from the published render graph, so an edit racing the callback can
//! neither skip a block nor count a lock miss.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use resonance_audio::test_support::{MixAudioHarness, STATE_MAP_COUNT};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const CLIPS: u64 = 500;

fn midi_clip(id: ClipId, track_id: TrackId) -> MidiClip {
    MidiClip {
        id,
        track_id,
        start_sample: 0,
        duration_ticks: 64 * TICKS_PER_QUARTER_NOTE,
        notes: (0..16)
            .map(|i| MidiNote {
                note: 60 + (i % 12) as u8,
                velocity: 0.8,
                start_tick: i * TICKS_PER_QUARTER_NOTE,
                duration_ticks: TICKS_PER_QUARTER_NOTE / 2,
            })
            .collect(),
        name: format!("clip {id}"),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

/// A minute of a constant 0.1 on `track_id`, from the top.
fn audio_clip(id: ClipId, track_id: TrackId) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.1; 2 * SR as usize * 60]),
        name: "a".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// A playing 8-track project with 500 MIDI clips and an audio clip, so
/// the playing branch takes every remaining lock and walks the graph.
fn playing_harness() -> MixAudioHarness {
    let tracks: Vec<Track> = (1..=8)
        .map(|id| {
            // Track 1 plays the audio clip; the rest are instrument
            // tracks whose MIDI the render walks every block.
            let t = if id == 1 {
                Track::new(id, "audio".into())
            } else {
                Track::with_type(id, format!("t{id}"), TrackType::Instrument)
            };
            t.set_output(TrackOutput::Master);
            t
        })
        .collect();
    let audio = audio_clip(1, 1);
    let midi: Vec<MidiClip> = (0..CLIPS).map(|i| midi_clip(10 + i, 1 + i % 8)).collect();
    let h = MixAudioHarness::new(
        tracks,
        Vec::new(),
        vec![audio],
        midi,
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

#[test]
fn midi_edits_published_while_the_callback_renders_never_skip_a_block() {
    let mut h = playing_harness();
    let shared = h.shared_arc();
    let stop = Arc::new(AtomicBool::new(false));

    // The engine thread: hammer note edits across the project — a note
    // drag, a velocity change, a bulk replace — publishing a new graph
    // each time, as fast as it can.
    let editor = {
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut edits = 0u64;
            while !stop.load(Ordering::Acquire) {
                let id = 10 + edits % CLIPS;
                let edited = shared.edit_midi_clip(id, |clip| match edits % 3 {
                    0 => clip.notes[0].start_tick = (edits % 7) * 10,
                    1 => clip.notes[1].velocity = (edits % 100) as f32 / 100.0,
                    _ => clip.notes = midi_clip(id, clip.track_id).notes,
                });
                assert!(edited.is_some());
                edits += 1;
                if edits.is_multiple_of(64) {
                    // The engine loop's 16 ms tick.
                    shared.retired.sweep();
                }
            }
            edits
        })
    };

    for block in 0..2_000 {
        let out = h.render();
        assert!(out.iter().any(|&s| s != 0.0), "block {block} rendered");
    }
    stop.store(true, Ordering::Release);
    let edits = editor.join().expect("editor");

    assert!(edits > 0, "the editor actually raced the callback");
    assert_eq!(
        h.shared().render_skip_cycles.load(Ordering::Relaxed),
        0,
        "no block was skipped across {edits} concurrent edits"
    );
    assert_eq!(h.shared().lock_misses.snapshot(), [0; STATE_MAP_COUNT]);
    // Every replaced graph is freed by an engine-side sweep once the
    // callback's last load let go of it.
    h.shared().retired.sweep();
    assert!(h.shared().retired.is_empty());
}

const DRUMS: BusId = 7;
const VERB: BusId = 8;
/// A bus the editor keeps adding and removing past the two routed ones.
const SCRATCH_BUS: BusId = 9;

/// A playing project with two audio tracks, each routed through its own
/// bus (non-unity fader and pan, so the bus pass and its gain ramp shape
/// the output), and an empty master chain.
fn bus_harness() -> MixAudioHarness {
    let tracks: Vec<Track> = [(1, DRUMS), (2, VERB)]
        .into_iter()
        .map(|(id, bus)| {
            let t = Track::new(id, format!("t{id}"));
            t.set_output(TrackOutput::Bus(bus));
            t
        })
        .collect();
    let busses: Vec<Bus> = [(DRUMS, 0.5, -0.3), (VERB, 0.8, 0.6)]
        .into_iter()
        .map(|(id, volume, pan)| {
            let b = Bus::new(id, format!("bus {id}"));
            b.set_volume(volume);
            b.set_pan(pan);
            b
        })
        .collect();
    let h = MixAudioHarness::new(
        tracks,
        busses,
        vec![audio_clip(1, 1), audio_clip(2, 2)],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

const BUS_BLOCKS: usize = 2_000;

fn render_blocks(h: &mut MixAudioHarness) -> Vec<f32> {
    let mut out = Vec::with_capacity(BUS_BLOCKS * BLOCK * 2);
    for block in 0..BUS_BLOCKS {
        let data = h.render();
        assert!(data.iter().any(|&s| s != 0.0), "block {block} rendered");
        out.extend_from_slice(data);
    }
    out
}

/// Bus and master edits — rename, insert-chain edits, a bus added and
/// removed, fader writes through the published bus — published as fast as
/// the editor can while the callback renders. None of them changes what
/// the project sounds like (the chain ids name no instance, so the chain
/// pass skips them; the scratch bus has no input), so the output must be
/// bit-identical to an unedited run: a bus copy that lost its live state
/// (the fader ramp's last gains) would show up as a restarted ramp.
#[test]
fn bus_and_master_edits_published_while_the_callback_renders_never_skip_a_block() {
    let reference = render_blocks(&mut bus_harness());

    let mut h = bus_harness();
    let shared = h.shared_arc();
    let stop = Arc::new(AtomicBool::new(false));
    let editor = {
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut edits = 0u64;
            while !stop.load(Ordering::Acquire) {
                let bus = if edits % 2 == 0 { DRUMS } else { VERB };
                match edits % 6 {
                    0 => {
                        shared
                            .edit_bus(bus, |b| b.name = format!("bus {bus} #{edits}"))
                            .expect("bus exists");
                    }
                    1 => {
                        shared
                            .edit_bus(bus, |b| b.plugin_ids.push(1_000 + edits))
                            .expect("bus exists");
                    }
                    2 => {
                        shared
                            .edit_bus(bus, |b| b.plugin_ids.clear())
                            .expect("bus exists");
                    }
                    3 => shared.edit_master(|m| m.plugin_ids.push(2_000 + edits)),
                    4 => shared.edit_master(|m| m.plugin_ids.clear()),
                    _ => {
                        shared.edit_busses(|busses| {
                            if busses.shift_remove(&SCRATCH_BUS).is_none() {
                                busses.insert(
                                    SCRATCH_BUS,
                                    Arc::new(Bus::new(SCRATCH_BUS, "scratch".into())),
                                );
                            }
                        });
                    }
                }
                // A fader "move" to the value it already has: written
                // through the published bus, no graph published.
                let graph = shared.graph.load();
                let drums = graph.bus(DRUMS).expect("routed bus never leaves");
                drums.set_volume(drums.volume());
                drop(graph);
                edits += 1;
                if edits.is_multiple_of(64) {
                    // The engine loop's 16 ms tick.
                    shared.retired.sweep();
                }
            }
            edits
        })
    };

    let hammered = render_blocks(&mut h);
    stop.store(true, Ordering::Release);
    let edits = editor.join().expect("editor");

    assert!(edits > 0, "the editor actually raced the callback");
    assert_eq!(
        h.shared().render_skip_cycles.load(Ordering::Relaxed),
        0,
        "no block was skipped across {edits} concurrent bus / master edits"
    );
    assert_eq!(h.shared().lock_misses.snapshot(), [0; STATE_MAP_COUNT]);
    assert!(
        hammered == reference,
        "the edits changed the rendered audio (first diff at sample {:?})",
        hammered.iter().zip(&reference).position(|(a, b)| a != b)
    );
    h.shared().retired.sweep();
    assert!(h.shared().retired.is_empty());
}
