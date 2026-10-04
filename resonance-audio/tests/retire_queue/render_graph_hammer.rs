//! The A2-9 hammer (code review ARCH-02, refactor todo B-6): Epic B's
//! done-when test.
//!
//! A 500-clip project — audio clips loaded through the real
//! `LoadClipFromWav` handler and import pool, MIDI clips with bulk note
//! writes on instrument tracks, busses, aux sends, and pass-through /
//! instrument probe plugins on tracks, busses and the master — plays
//! through the real audio callback (`MixAudioHarness::on_shared`) on its
//! own thread, while this thread plays the engine loop: it drives heavy
//! edits through the real top-level dispatch (clip load / delete / move /
//! trim / split / fade / gain, MIDI clip create / move / delete and bulk
//! note replaces, track / bus add / remove / re-route, sends, plugin add /
//! remove / move / bypass, `ClearAll` + reload), applies worker results,
//! and sweeps the retire queue on its tick.
//!
//! What must hold:
//!
//! - **No silent block where audio is expected.** A base track plays a
//!   constant clip no edit touches (its route and its plugin chain do
//!   change), and every other source is a positive constant too, so every
//!   sample of every block rendered outside a `ClearAll` + reload window
//!   must carry at least the base level — not merely be non-zero: the
//!   other tracks alone would pass that. The one exception is the first
//!   block after the base track is (re)created, whose gain ramps in from
//!   silence. There is no skip path any more (B-6 deleted it with the
//!   lock-miss counters); a contended or torn graph, or a route edit that
//!   drops the base track for a block, would show up here.
//! - **Zero retire-queue drops on the render thread.** The callback frees
//!   nothing: every replaced graph, clip, chain and plugin slot is owned by
//!   the retire queue until the engine-side sweep — counted with this
//!   binary's thread-local free counter, across every `render()` call.
//! - **Every plugin is destroyed on the sweeping thread**, exactly once,
//!   and the retire queue drains to empty at the end.

use std::collections::BTreeSet;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    EngineHandlerHarness, MixAudioHarness, PluginSlot, SharedState, __instance_from_raw_for_test,
};
use resonance_audio::transcode_to_wav;
use resonance_audio::types::*;

use crate::deallocs_here;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
/// The loop the transport cycles: two seconds, so the seam is crossed
/// every 750 blocks. A whole number of blocks, so every seam is the
/// aligned kind (a full head, a zero-frame tail sub-render).
const LOOP_LEN: u64 = 2 * SR as u64;
/// Where the loop starts: a block-aligned point the base clip is already
/// past its head at. A loop starting on the base clip's first frame
/// re-enters its head every pass, and a clip head always gets the
/// `CLIP_DECLICK_FRAMES` anti-click ramp — so the first frame after each
/// wrap would carry none of the base level, by design (FU-B6b,
/// `tests/mixer/bus_first_block_loop_seam.rs`).
const LOOP_IN: u64 = 64 * BLOCK as u64;

const BASE_TRACK: TrackId = 1;
const BASE_CLIP: ClipId = 1;
/// The base clip's constant level: through unity faders, centre balance
/// and pass-through inserts it reaches the output unchanged.
const BASE_LEVEL: f32 = 0.1;
/// Every other source is a positive constant far below it: at most ~600
/// loaded clips at `LOAD_LEVEL`, eight instruments at `INSTRUMENT_LEVEL`
/// and [`MAX_SENDS`] -6 dB sends of either sum to < 0.04, so a block
/// without the base track reads well under `BASE_LEVEL` and one with it
/// never does.
const LOAD_LEVEL: f32 = 2e-5;
const INSTRUMENT_LEVEL: f32 = 1e-3;
/// Live aux sends are capped here (FU-B6b). Unbounded, they piled up to
/// ~1,700 over a run — an instrument track with a few hundred sends alone
/// outweighs the base level — and the floor check below stopped being able
/// to tell the base track from everything else.
const MAX_SENDS: usize = 32;
/// Audio tracks 2..=16, instrument tracks 17..=24.
const AUDIO_TRACKS: std::ops::RangeInclusive<TrackId> = 2..=16;
const INSTRUMENT_TRACKS: std::ops::RangeInclusive<TrackId> = 17..=24;
const BUSSES: std::ops::RangeInclusive<BusId> = 1..=4;
/// 420 loaded audio clips + 79 MIDI clips + the base clip = 500.
const LOADED_CLIPS: u64 = 420;
const MIDI_CLIPS: u64 = 79;
/// Each hammer phase (two, around a `ClearAll` + reload) runs at least
/// this many edits and lasts at least this many rendered blocks — the
/// editor outpaces the callback by an order of magnitude, so it is the
/// block count that sets the run time.
const EDITS: u64 = 1_500;
const PHASE_BLOCKS: u64 = 800;

// ---------------------------------------------------------------------------
// Probe plugins: every `destroy` logs its thread.
// ---------------------------------------------------------------------------

/// The threads each probe's `destroy` ran on, in order.
type DestroyLog = Arc<Mutex<Vec<ThreadId>>>;

struct Probe {
    log: DestroyLog,
    /// Input events seen by `process` (instrument probes: the MIDI notes).
    events: Arc<AtomicU64>,
    instrument: bool,
}

unsafe extern "C" fn p_init(_p: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn p_destroy(p: *const clap_plugin) {
    unsafe {
        let probe = Box::from_raw((*p).plugin_data as *mut Probe);
        probe.log.lock().unwrap().push(std::thread::current().id());
        drop(Box::from_raw(p as *mut clap_plugin));
    }
}

unsafe extern "C" fn p_activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}
unsafe extern "C" fn p_deactivate(_p: *const clap_plugin) {}
unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn p_stop(_p: *const clap_plugin) {}
unsafe extern "C" fn p_reset(_p: *const clap_plugin) {}

/// An effect probe passes its input through; an instrument probe counts
/// its input events and writes a quiet DC, so an instrument track sounds
/// whatever its MIDI clips hold.
unsafe extern "C" fn p_process(
    p: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    unsafe {
        let probe = &*((*p).plugin_data as *const Probe);
        let frames = (*process).frames_count as usize;
        let out = &*(*process).audio_outputs;
        if probe.instrument {
            let events = (*process).in_events;
            if !events.is_null() {
                if let Some(size) = (*events).size {
                    probe.events.fetch_add(size(events) as u64, Ordering::Relaxed);
                }
            }
            for ch in 0..2 {
                let dst = *out.data32.add(ch);
                std::slice::from_raw_parts_mut(dst, frames).fill(INSTRUMENT_LEVEL);
            }
        } else {
            let inp = &*(*process).audio_inputs;
            for ch in 0..2 {
                let src = *inp.data32.add(ch) as *const f32;
                let dst = *out.data32.add(ch);
                if !ptr::eq(src, dst) {
                    ptr::copy_nonoverlapping(src, dst, frames);
                }
            }
        }
    }
    CLAP_PROCESS_CONTINUE
}

/// Makes probe instances and keeps the books on them.
struct Probes {
    log: DestroyLog,
    events: Arc<AtomicU64>,
    created: u64,
    next_id: PluginInstanceId,
}

impl Probes {
    fn new() -> Self {
        Self {
            log: DestroyLog::default(),
            events: Arc::new(AtomicU64::new(0)),
            created: 0,
            next_id: 1_000,
        }
    }

    fn slot(&mut self, instrument: bool) -> (PluginInstanceId, Arc<PluginSlot>) {
        let log = Arc::clone(&self.log);
        let events = Arc::clone(&self.events);
        let inst = __instance_from_raw_for_test(
            move |_host| {
                let probe = Box::into_raw(Box::new(Probe {
                    log,
                    events,
                    instrument,
                }));
                Box::into_raw(Box::new(clap_plugin {
                    desc: ptr::null(),
                    plugin_data: probe as *mut c_void,
                    init: Some(p_init),
                    destroy: Some(p_destroy),
                    activate: Some(p_activate),
                    deactivate: Some(p_deactivate),
                    start_processing: Some(p_start),
                    stop_processing: Some(p_stop),
                    reset: Some(p_reset),
                    process: Some(p_process),
                    get_extension: None,
                    on_main_thread: None,
                })) as *const clap_plugin
            },
            SR,
        )
        .expect("probe plugin instance");
        self.created += 1;
        self.next_id += 1;
        (self.next_id, Arc::new(PluginSlot::new(inst)))
    }

    /// `AddPlugin`'s publish order (`engine/plugins.rs`), with a probe in
    /// place of a CLAP bundle: the slot first, then its id on the chain.
    fn add_to_track(&mut self, shared: &SharedState, track_id: TrackId, instrument: bool) {
        let Some(track) = shared.tracks().get(&track_id).cloned() else {
            return;
        };
        let (id, slot) = self.slot(instrument);
        shared.edit_plugins(|p| p.insert(id, slot));
        shared.retired.retire(track.push_plugin(id));
    }

    /// A bus `AddPlugin`'s publish order (`engine/chain.rs`).
    fn add_to_bus(&mut self, shared: &SharedState, bus_id: BusId) {
        if shared.graph.load().bus(bus_id).is_none() {
            return;
        }
        let (id, slot) = self.slot(false);
        shared.edit_plugins(|p| p.insert(id, slot));
        shared.edit_bus(bus_id, |b| b.plugin_ids.push(id));
    }

    /// A master `AddPlugin`'s publish order.
    fn add_to_master(&mut self, shared: &SharedState) {
        let (id, slot) = self.slot(false);
        shared.edit_plugins(|p| p.insert(id, slot));
        shared.edit_master(|m| m.plugin_ids.push(id));
    }

    fn destroyed(&self) -> Vec<ThreadId> {
        self.log.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------

/// A constant `level` on `track_id`, longer than the loop, from the top.
fn memory_clip(id: ClipId, track_id: TrackId, level: f32) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![level; 2 * (LOOP_LEN as usize + SR as usize)]),
        name: format!("clip {id}"),
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

/// Half a second of a quiet constant: what every `LoadClipFromWav` reads.
fn wav() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "resonance-a2-9-hammer-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("load.wav");
    transcode_to_wav(&path, &vec![LOAD_LEVEL; SR as usize], SR).expect("write test wav");
    (dir, path)
}

/// `count` eighth notes from the top of a one-bar clip, pitched by `seed`.
fn notes(count: u64, seed: u64) -> Vec<MidiNote> {
    let eighth = TICKS_PER_QUARTER_NOTE / 2;
    (0..count)
        .map(|i| MidiNote {
            note: (36 + (seed + i * 7) % 48) as u8,
            velocity: 0.5 + ((seed + i) % 5) as f32 / 10.0,
            start_tick: (i % 8) * eighth,
            duration_ticks: eighth - 10,
        })
        .collect()
}

/// The engine loop around the handlers: ids, the WAV, and the books.
struct Engine {
    h: EngineHandlerHarness,
    probes: Probes,
    wav: PathBuf,
    next_clip: ClipId,
    next_track: TrackId,
    next_bus: BusId,
    next_send: SendId,
    loads: u64,
    edits: u64,
    /// Blocks the callback has rendered.
    rendered: Arc<AtomicU64>,
    /// Times the base track was routed onto a scratch bus, and times a
    /// `RemoveBus` re-routed it back to the master.
    base_via_bus: u64,
    base_rerouted: u64,
}

impl Engine {
    fn shared(&self) -> &SharedState {
        self.h.shared()
    }

    fn dispatch(&mut self, cmd: AudioCommand) {
        self.h.dispatch(cmd);
        self.edits += 1;
        self.tick();
    }

    /// One engine-loop pass: worker results, parked edits, the event
    /// channel, and — every 16th pass — the retire sweep.
    fn tick(&mut self) {
        self.h.poll_deferred_clip_commands();
        if self.edits.is_multiple_of(16) {
            self.h.sweep_retired();
        }
        self.h.drain_events();
    }

    fn load_clip(&mut self, track_id: TrackId, start_sample: u64) {
        self.next_clip += 1;
        self.loads += 1;
        self.h.load_clip_from_wav(
            self.next_clip,
            track_id,
            start_sample,
            self.wav.clone(),
            "load".into(),
        );
        self.edits += 1;
        self.tick();
    }

    fn create_midi_clip(&mut self, track_id: TrackId, start_sample: u64, seed: u64) {
        self.next_clip += 1;
        let clip_id = self.next_clip;
        self.dispatch(AudioCommand::CreateMidiClip {
            clip_id,
            track_id,
            start_sample,
            duration_ticks: 4 * TICKS_PER_QUARTER_NOTE,
            name: "midi".into(),
        });
        self.dispatch(AudioCommand::SetMidiClipNotes {
            clip_id,
            notes: notes(16, seed),
        });
    }

    /// The base track and its clip: what every block plays. Published
    /// before the renderer starts, and again by every reload.
    fn build_base(&mut self) {
        self.dispatch(AudioCommand::AddTrack {
            id: BASE_TRACK,
            name: Some("base".into()),
        });
        let shared = self.h.shared_arc();
        self.probes.add_to_track(&shared, BASE_TRACK, false);
        self.probes.add_to_track(&shared, BASE_TRACK, false);
        self.probes.add_to_master(&shared);
        self.h.push_clip(memory_clip(BASE_CLIP, BASE_TRACK, BASE_LEVEL));
    }

    /// Everything else: tracks, busses, sends, plugins and 499 clips —
    /// the audio ones through the import pool, landing while the callback
    /// renders.
    fn build_rest(&mut self) {
        let shared = self.h.shared_arc();
        for id in AUDIO_TRACKS {
            self.dispatch(AudioCommand::AddTrack { id, name: None });
            if id % 2 == 0 {
                self.probes.add_to_track(&shared, id, false);
            }
        }
        for id in INSTRUMENT_TRACKS {
            self.dispatch(AudioCommand::AddInstrumentTrack { id, name: None });
            self.probes.add_to_track(&shared, id, true);
        }
        for id in BUSSES {
            self.dispatch(AudioCommand::AddBus { id, name: None });
            self.probes.add_to_bus(&shared, id);
        }
        for (i, track_id) in AUDIO_TRACKS.enumerate() {
            let dest = 1 + i as u64 % 4;
            if i % 3 == 0 {
                self.dispatch(AudioCommand::SetTrackOutput {
                    track_id,
                    output: TrackOutput::Bus(dest),
                });
            } else {
                self.add_send(track_id, dest);
            }
        }
        let tracks: Vec<TrackId> = AUDIO_TRACKS.collect();
        for i in 0..LOADED_CLIPS {
            let track = tracks[i as usize % tracks.len()];
            self.load_clip(track, (i * 997) % LOOP_LEN);
        }
        let instruments: Vec<TrackId> = INSTRUMENT_TRACKS.collect();
        for i in 0..MIDI_CLIPS {
            let track = instruments[i as usize % instruments.len()];
            self.create_midi_clip(track, (i * 1_231) % (LOOP_LEN / 2), i);
        }
    }

    /// Add a -6 dB post-fader send, unless [`MAX_SENDS`] are already live.
    fn add_send(&mut self, track_id: TrackId, dest: BusId) {
        if self.shared().aux_sends.load().len() >= MAX_SENDS {
            return;
        }
        self.next_send += 1;
        self.dispatch(AudioCommand::AddAuxSend {
            id: self.next_send,
            source: SendSource::Track(track_id),
            dest,
            level_db: -6.0,
            pre_fader: false,
            enabled: true,
        });
    }

    /// Run engine-loop passes until every load in flight has landed.
    fn settle_loads(&mut self, expected_clips: usize) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.shared().clips().len() < expected_clips {
            assert!(
                Instant::now() < deadline,
                "loads never landed: {} of {expected_clips} clips",
                self.shared().clips().len()
            );
            self.tick();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// One heavy edit, chosen by `k`.
    fn edit(&mut self, k: u64) {
        let graph = self.h.render_graph();
        let clips: Vec<ClipId> = graph.clips.iter().map(|c| c.id).filter(|&id| id != BASE_CLIP).collect();
        let midi: Vec<ClipId> = graph.midi_clips.iter().map(|c| c.id).collect();
        let tracks: Vec<TrackId> = graph.tracks.keys().copied().filter(|&id| id != BASE_TRACK).collect();
        let busses: Vec<BusId> = graph.busses.keys().copied().collect();
        let plugins: Vec<(TrackId, PluginInstanceId)> = graph
            .tracks
            .values()
            .flat_map(|t| t.plugins().iter().map(|&p| (t.id, p)).collect::<Vec<_>>())
            .collect();
        let bus_plugins: Vec<(BusId, PluginInstanceId)> = graph
            .busses
            .values()
            .flat_map(|b| b.plugin_ids.iter().map(|&p| (b.id, p)).collect::<Vec<_>>())
            .collect();
        drop(graph);
        // Bounds, so a long run stays a ~500-clip project: splits and
        // loads stop at 600 clips (deletes start above 300), plugin adds
        // at 150 instances.
        let full = clips.len() >= 600;
        let scratch_busses: Vec<BusId> =
            busses.iter().copied().filter(|&b| b > *BUSSES.end()).collect();
        let crowded = plugins.len() + bus_plugins.len() >= 150;
        let pick = |v: &[u64]| v.get((k as usize * 31) % v.len().max(1)).copied();
        let shared = self.h.shared_arc();

        match k % 24 {
            0 => {
                if let Some(clip_id) = pick(&clips) {
                    let new_track_id = pick(&tracks).unwrap_or(BASE_TRACK);
                    self.dispatch(AudioCommand::MoveClip {
                        clip_id,
                        new_start_sample: (k * 331) % LOOP_LEN,
                        new_track_id,
                    });
                }
            }
            1 => {
                if let Some(clip_id) = pick(&clips) {
                    self.dispatch(AudioCommand::TrimClip {
                        clip_id,
                        new_start_sample: (k * 17) % LOOP_LEN,
                        trim_start_frames: k % 2_000,
                        trim_end_frames: k % 1_000,
                    });
                }
            }
            2 if !full => {
                if let Some(clip_id) = pick(&clips) {
                    self.next_clip += 1;
                    let new_clip_id = self.next_clip;
                    self.dispatch(AudioCommand::SplitClip {
                        clip_id,
                        new_clip_id,
                        at_sample: (k * 53) % LOOP_LEN,
                    });
                }
            }
            3 if clips.len() > 300 => {
                let clip_id = pick(&clips).expect("clips");
                self.dispatch(AudioCommand::DeleteClip { clip_id });
            }
            4 if !full => {
                let track = pick(&tracks).unwrap_or(BASE_TRACK);
                self.load_clip(track, (k * 211) % LOOP_LEN);
            }
            5 => {
                if let Some(clip_id) = pick(&clips) {
                    self.dispatch(AudioCommand::SetClipFade {
                        clip_id,
                        fade_in_frames: k % 500,
                        fade_in_curve: FadeCurve::EqualPower,
                        fade_out_frames: k % 700,
                        fade_out_curve: FadeCurve::Linear,
                    });
                    self.dispatch(AudioCommand::SetClipGain {
                        clip_id,
                        gain_db: -((k % 12) as f32),
                    });
                }
            }
            6 | 7 => {
                if let Some(clip_id) = pick(&midi) {
                    self.dispatch(AudioCommand::SetMidiClipNotes {
                        clip_id,
                        notes: notes(8 + k % 64, k),
                    });
                }
            }
            8 => {
                if let Some(clip_id) = pick(&midi) {
                    let instruments: Vec<TrackId> = INSTRUMENT_TRACKS.collect();
                    self.dispatch(AudioCommand::MoveMidiClip {
                        clip_id,
                        new_start_sample: (k * 97) % (LOOP_LEN / 2),
                        new_track_id: instruments[k as usize % instruments.len()],
                    });
                }
            }
            9 => {
                if midi.len() > 40 {
                    let clip_id = pick(&midi).expect("midi");
                    self.dispatch(AudioCommand::DeleteMidiClip { clip_id });
                }
                let instruments: Vec<TrackId> = INSTRUMENT_TRACKS.collect();
                let track = instruments[k as usize % instruments.len()];
                if shared.tracks().contains_key(&track) {
                    self.create_midi_clip(track, (k * 41) % (LOOP_LEN / 2), k);
                }
            }
            10 if !crowded => {
                // A scratch track with a plugin and a send, and a clip
                // moved onto it.
                self.next_track += 1;
                let id = self.next_track;
                self.dispatch(AudioCommand::AddTrack { id, name: None });
                self.probes.add_to_track(&shared, id, false);
                if let Some(dest) = pick(&busses) {
                    self.add_send(id, dest);
                }
                if let Some(clip_id) = pick(&clips) {
                    self.dispatch(AudioCommand::MoveClip {
                        clip_id,
                        new_start_sample: (k * 7) % LOOP_LEN,
                        new_track_id: id,
                    });
                }
            }
            11 => {
                // Remove a scratch track (plugins, sends and clips on it go).
                let scratch: Vec<TrackId> =
                    tracks.iter().copied().filter(|&t| t > *INSTRUMENT_TRACKS.end()).collect();
                if let Some(track_id) = pick(&scratch) {
                    self.dispatch(AudioCommand::RemoveTrack { track_id });
                }
            }
            12 => {
                let born = (scratch_busses.len() < 6).then(|| {
                    self.next_bus += 1;
                    let id = self.next_bus;
                    self.dispatch(AudioCommand::AddBus { id, name: None });
                    if !crowded {
                        self.probes.add_to_bus(&shared, id);
                    }
                    id
                });
                // The base track itself goes through a scratch bus: the
                // one born this pass when there is one, so the bus's very
                // first rendered block often carries it — which must not
                // dip (FU-B6a; `tests/mixer/bus_first_block_gain.rs`,
                // `bus_first_block_loop_seam.rs`).
                if let Some(id) = born.or_else(|| scratch_busses.last().copied()) {
                    self.dispatch(AudioCommand::SetTrackOutput {
                        track_id: BASE_TRACK,
                        output: TrackOutput::Bus(id),
                    });
                    self.base_via_bus += 1;
                }
            }
            13 => {
                // Remove a scratch bus: its feeders (maybe the base track)
                // are re-routed to the master in the same graph.
                // Keep a few alive, so some age past their first block;
                // alternately the oldest (the one the base track is routed
                // to, if any) and the youngest.
                let victim = if (k / 24) % 2 == 0 {
                    scratch_busses.first()
                } else {
                    scratch_busses.last()
                };
                if let Some(&bus_id) = victim.filter(|_| scratch_busses.len() > 3) {
                    let feeds_it = shared
                        .tracks()
                        .get(&BASE_TRACK)
                        .is_some_and(|t| t.output() == TrackOutput::Bus(bus_id));
                    self.dispatch(AudioCommand::RemoveBus { bus_id });
                    self.base_rerouted += u64::from(feeds_it);
                }
            }
            14 => {
                if let Some(track_id) = pick(&tracks) {
                    let output = match pick(&busses) {
                        Some(b) if k % 3 != 0 => TrackOutput::Bus(b),
                        _ => TrackOutput::Master,
                    };
                    self.dispatch(AudioCommand::SetTrackOutput { track_id, output });
                }
            }
            15 => {
                if let (Some(track_id), Some(dest)) = (pick(&tracks), pick(&busses)) {
                    self.add_send(track_id, dest);
                }
            }
            16 => {
                if self.next_send > 0 {
                    let send_id = 1 + (k * 13) % self.next_send;
                    self.dispatch(AudioCommand::RemoveAuxSend { send_id });
                }
            }
            17 if !crowded => {
                let track = pick(&tracks).unwrap_or(BASE_TRACK);
                self.probes.add_to_track(&shared, track, false);
                self.edits += 1;
            }
            18 => {
                // Never the base track's last pass-through, never an
                // instrument (first on an instrument track).
                let removable: Vec<(TrackId, PluginInstanceId)> = plugins
                    .iter()
                    .copied()
                    .filter(|&(t, p)| {
                        let chain = shared.tracks().get(&t).map(|tr| tr.plugins().as_ref().clone());
                        chain.is_some_and(|c| c.len() > 1 && c.first() != Some(&p))
                    })
                    .collect();
                if let Some(&(track_id, instance_id)) =
                    removable.get((k as usize * 7) % removable.len().max(1))
                {
                    self.dispatch(AudioCommand::RemovePlugin {
                        owner: ChainOwner::Track(track_id),
                        instance_id,
                    });
                }
            }
            19 => {
                if let Some(&(track_id, instance_id)) =
                    plugins.get((k as usize * 11) % plugins.len().max(1))
                {
                    self.dispatch(AudioCommand::MovePlugin {
                        owner: ChainOwner::Track(track_id),
                        instance_id,
                        to_index: (k % 3) as usize,
                    });
                }
            }
            20 => {
                if let Some(&(_, instance_id)) = plugins.get((k as usize * 5) % plugins.len().max(1)) {
                    self.dispatch(AudioCommand::SetPluginBypass {
                        instance_id,
                        bypassed: k % 4 == 0,
                    });
                }
            }
            21 if !crowded => {
                if let Some(bus_id) = pick(&busses) {
                    self.probes.add_to_bus(&shared, bus_id);
                    self.edits += 1;
                }
            }
            22 => {
                if let Some(&(bus_id, instance_id)) =
                    bus_plugins.get((k as usize * 3) % bus_plugins.len().max(1))
                {
                    if k % 2 == 0 {
                        self.dispatch(AudioCommand::RemovePlugin {
                            owner: ChainOwner::Bus(bus_id),
                            instance_id,
                        });
                    } else {
                        self.dispatch(AudioCommand::MovePlugin {
                            owner: ChainOwner::Bus(bus_id),
                            instance_id,
                            to_index: 0,
                        });
                    }
                }
            }
            _ => {
                // The base track back to the master.
                self.dispatch(AudioCommand::SetTrackOutput {
                    track_id: BASE_TRACK,
                    output: TrackOutput::Master,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The hammer
// ---------------------------------------------------------------------------

/// What the render thread saw.
#[derive(Debug, Default)]
struct Rendered {
    blocks: u64,
    /// Blocks rendered wholly outside a `ClearAll` + reload window.
    checked: u64,
    silent: u64,
    first_silent: Option<u64>,
    /// Checked blocks with a sample below the base level (the base track
    /// dropped out for part of the block), and the lowest such sample.
    dips: u64,
    first_dip: Option<(u64, f32)>,
    /// Checked blocks that wrapped the loop, so the floor above was held
    /// across a seam too.
    wraps: u64,
    /// Heap frees inside `render()` — a retired value dropped by the
    /// callback would land here.
    frees: u64,
    thread: Option<ThreadId>,
}

#[test]
fn a_500_clip_project_under_heavy_edits_renders_every_block_and_frees_nothing_on_the_render_thread() {
    let (dir, wav) = wav();
    let mut engine = Engine {
        h: EngineHandlerHarness::new(),
        probes: Probes::new(),
        wav,
        next_clip: 1_000,
        next_track: 100,
        next_bus: 100,
        next_send: 0,
        loads: 0,
        edits: 0,
        rendered: Arc::new(AtomicU64::new(0)),
        base_via_bus: 0,
        base_rerouted: 0,
    };
    let shared = engine.h.shared_arc();
    let arm = |shared: &SharedState| {
        shared.set_loop_range(resonance_audio::test_support::LoopRange::new(true, LOOP_IN, LOOP_IN + LOOP_LEN));
        shared.playing.store(true, Ordering::Relaxed);
    };
    engine.build_base();
    arm(&shared);

    // Even: the project is whole and the base clip must sound. Odd: a
    // `ClearAll` + reload is in progress (a seqlock the renderer brackets
    // each block with).
    let epoch = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let rendered = Arc::clone(&engine.rendered);
    let renderer = {
        let shared = Arc::clone(&shared);
        let epoch = Arc::clone(&epoch);
        let stop = Arc::clone(&stop);
        let rendered = Arc::clone(&rendered);
        std::thread::spawn(move || {
            let transport = Arc::clone(&shared);
            let mut cb = MixAudioHarness::on_shared(shared, BLOCK, 2, SR);
            let mut seen = Rendered {
                thread: Some(std::thread::current().id()),
                ..Rendered::default()
            };
            let mut ramped_in: Option<u64> = None;
            while !stop.load(Ordering::Acquire) {
                let before_epoch = epoch.load(Ordering::Acquire);
                let before_playhead = transport.playhead.load(Ordering::Acquire);
                let before_frees = deallocs_here();
                let out = cb.render();
                seen.frees += deallocs_here() - before_frees;
                if before_epoch % 2 == 0 && epoch.load(Ordering::Acquire) == before_epoch {
                    seen.checked += 1;
                    if transport.playhead.load(Ordering::Acquire) < before_playhead {
                        seen.wraps += 1;
                    }
                    if !out.iter().any(|&s| s != 0.0) {
                        seen.silent += 1;
                        seen.first_silent.get_or_insert(seen.blocks);
                    }
                    // The first checked block of an epoch may ramp the
                    // freshly created base track in from silence.
                    let low = out.iter().fold(f32::MAX, |m, &s| m.min(s));
                    if ramped_in == Some(before_epoch) && low < BASE_LEVEL * 0.999 {
                        seen.dips += 1;
                        seen.first_dip.get_or_insert((seen.blocks, low));
                    }
                    ramped_in = Some(before_epoch);
                }
                seen.blocks += 1;
                rendered.store(seen.blocks, Ordering::Release);
            }
            seen
        })
    };

    let started = Instant::now();
    // Load the project under render.
    engine.build_rest();
    engine.settle_loads(1 + LOADED_CLIPS as usize);
    assert_eq!(engine.shared().graph.load().midi_clips.len(), MIDI_CLIPS as usize);
    assert_eq!(
        engine.shared().clips().len() + engine.shared().graph.load().midi_clips.len(),
        500,
        "a 500-clip project"
    );

    // Heavy edits for a phase: at least `EDITS` of them, and until the
    // callback has rendered `PHASE_BLOCKS` more blocks.
    let mut k = 0u64;
    let mut hammer = |engine: &mut Engine| {
        let target = rendered.load(Ordering::Acquire) + PHASE_BLOCKS;
        let deadline = Instant::now() + Duration::from_secs(60);
        let first = k;
        while k - first < EDITS || rendered.load(Ordering::Acquire) < target {
            assert!(Instant::now() < deadline, "the render thread stalled");
            engine.edit(k);
            k += 1;
        }
    };
    hammer(&mut engine);

    // `ClearAll` + reload, as a project load does.
    epoch.fetch_add(1, Ordering::AcqRel);
    engine.dispatch(AudioCommand::ClearAll);
    engine.build_base();
    arm(&shared);
    epoch.fetch_add(1, Ordering::AcqRel);
    engine.build_rest();
    engine.settle_loads(1 + LOADED_CLIPS as usize);

    hammer(&mut engine);
    let edit_time = started.elapsed();

    stop.store(true, Ordering::Release);
    let seen = renderer.join().expect("renderer");

    // The engine winds down: nothing pins a graph any more, so the last
    // sweeps free everything the edits retired.
    engine.dispatch(AudioCommand::ClearAll);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !engine.shared().retired.is_empty() && Instant::now() < deadline {
        engine.h.sweep_retired();
    }
    let _ = std::fs::remove_dir_all(&dir);

    eprintln!(
        "A2-9 hammer: {} edits ({} clip loads) in {edit_time:?}; {} blocks rendered, {} checked \
         ({} wrapping the loop); {} probes created; {} MIDI events delivered; base via a scratch \
         bus {}x, re-routed by RemoveBus {}x",
        engine.edits,
        engine.loads,
        seen.blocks,
        seen.checked,
        seen.wraps,
        engine.probes.created,
        engine.probes.events.load(Ordering::Relaxed),
        engine.base_via_bus,
        engine.base_rerouted,
    );
    assert!(
        seen.checked > 1_000,
        "the callback rendered through the edits ({} checked blocks)",
        seen.checked
    );
    assert!(seen.wraps > 0, "no checked block wrapped the loop");
    assert_eq!(
        seen.silent, 0,
        "{} of {} checked blocks were silent (first: block {:?})",
        seen.silent, seen.checked, seen.first_silent
    );
    assert_eq!(
        seen.dips, 0,
        "{} of {} checked blocks dropped below the base track's level (first: block, sample {:?})",
        seen.dips, seen.checked, seen.first_dip
    );
    assert_eq!(
        seen.frees, 0,
        "the render thread freed {} allocations across {} blocks",
        seen.frees, seen.blocks
    );
    assert!(
        engine.shared().retired.is_empty(),
        "the retire queue drained ({} entries left)",
        engine.shared().retired.len()
    );
    assert!(engine.probes.events.load(Ordering::Relaxed) > 0, "MIDI notes reached the instruments");
    assert!(
        engine.base_via_bus > 0 && engine.base_rerouted > 0,
        "the base track went through scratch busses ({}) and a bus removal re-routed it ({})",
        engine.base_via_bus,
        engine.base_rerouted
    );
    let destroyed = engine.probes.destroyed();
    assert_eq!(
        destroyed.len() as u64,
        engine.probes.created,
        "every probe destroyed exactly once"
    );
    let threads: BTreeSet<String> = destroyed.iter().map(|t| format!("{t:?}")).collect();
    assert_eq!(
        threads,
        BTreeSet::from([format!("{:?}", std::thread::current().id())]),
        "a plugin was destroyed off the sweeping (engine) thread"
    );
    assert_ne!(seen.thread, Some(std::thread::current().id()));
}
