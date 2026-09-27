//! Plugin instances in the published render graph (code review ARCH-02
//! A2-7, refactor todo B-4).
//!
//! Two properties:
//!
//! - **Drop discipline.** A removed plugin's `ClapInstance::drop`
//!   (editor teardown, deactivate, `destroy`) must never run on the audio
//!   thread. The graph a block pinned keeps a removed slot alive; when
//!   that block lets go, the engine's retire queue still shares the slot,
//!   so the destructor runs in the engine loop's sweep — never in the
//!   reader. Pinned here with a fake plugin whose `destroy` records the
//!   thread it ran on.
//! - **No skipped block.** Plugin add / remove / reorder / bypass
//!   published while the callback renders can neither skip a block nor
//!   count a lock miss (the callback used to `try_read` the plugin map),
//!   and — the edits being inaudible by construction — leave the output
//!   bit-identical to an unedited run.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    EngineHandlerHarness, MixAudioHarness, PluginSlot, SharedState, STATE_MAP_COUNT,
    __instance_from_raw_for_test,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const BLOCKS: usize = 2_000;

/// The threads each probe plugin's `destroy` ran on, in order.
type DestroyLog = Arc<Mutex<Vec<ThreadId>>>;

// ---------------------------------------------------------------------------
// Fake plugin: stereo pass-through whose `destroy` logs its thread.
// ---------------------------------------------------------------------------

struct Probe {
    log: DestroyLog,
}

unsafe extern "C" fn p_init(_p: *const clap_plugin) -> bool {
    true
}

/// The last call a `ClapInstance` makes on its plugin: log the thread,
/// then free what `probe_slot` leaked.
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

/// Output = input, sample for sample: an inaudible insert.
unsafe extern "C" fn p_process(
    _p: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    unsafe {
        let frames = (*process).frames_count as usize;
        let out = &*(*process).audio_outputs;
        let inp = &*(*process).audio_inputs;
        for ch in 0..2 {
            let src = *inp.data32.add(ch) as *const f32;
            let dst = *out.data32.add(ch);
            if !ptr::eq(src, dst) {
                ptr::copy_nonoverlapping(src, dst, frames);
            }
        }
    }
    CLAP_PROCESS_CONTINUE
}

fn probe_slot(log: &DestroyLog) -> PluginSlot {
    let log = Arc::clone(log);
    let inst = __instance_from_raw_for_test(
        move |_host| {
            let probe = Box::into_raw(Box::new(Probe { log }));
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
    PluginSlot::new(inst)
}

fn destroyed(log: &DestroyLog) -> Vec<ThreadId> {
    log.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Drop discipline
// ---------------------------------------------------------------------------

const PROBE: PluginInstanceId = 7;

/// What a reader keeps of the graph while the plugin is removed.
#[derive(Debug, Clone, Copy)]
enum Pin {
    /// The audio callback's shape: one `load()` guard for the block.
    Guard,
    /// An offline chunk's shape: an owned `Arc<RenderGraph>`.
    Graph,
    /// A clone of the slot itself, outliving the graph it came from.
    Slot,
}

/// A reader thread pins `shared`'s current graph the way `pin` says,
/// reports it is pinned, waits for `release`, drops its pin and returns
/// its thread id. It never sweeps — like the audio callback, it only ever
/// releases what it pinned.
fn spawn_reader(
    shared: Arc<SharedState>,
    pin: Pin,
) -> (mpsc::Receiver<()>, mpsc::Sender<()>, std::thread::JoinHandle<ThreadId>) {
    let (pinned_tx, pinned_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let reader = std::thread::spawn(move || {
        let keep: Box<dyn std::any::Any> = match pin {
            Pin::Guard => {
                // Held on this frame, as the callback holds it for a block.
                let guard = shared.graph.load();
                assert!(guard.plugin(PROBE).is_some());
                pinned_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                drop(guard);
                return std::thread::current().id();
            }
            Pin::Graph => Box::new(shared.graph.load_full()),
            Pin::Slot => Box::new(Arc::clone(
                shared.graph.load().plugins.get(&PROBE).expect("probe slot"),
            )),
        };
        pinned_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        drop(keep);
        std::thread::current().id()
    });
    (pinned_rx, release_tx, reader)
}

/// Sweep as the engine loop does, `n` ticks.
fn sweep(shared: &SharedState, n: usize) {
    for _ in 0..n {
        shared.retired.sweep();
    }
}

#[test]
fn a_removed_plugin_a_reader_pins_is_destroyed_by_the_engine_sweep_never_by_the_reader() {
    for pin in [Pin::Guard, Pin::Graph, Pin::Slot] {
        let log = DestroyLog::default();
        let shared = Arc::new(SharedState::default());
        let slot = Arc::new(probe_slot(&log));
        shared.edit_plugins(|p| p.insert(PROBE, slot));

        let (pinned, release, reader) = spawn_reader(Arc::clone(&shared), pin);
        pinned.recv().unwrap();

        // The engine removes it and keeps ticking: the reader still pins
        // it, so no sweep may destroy it yet.
        shared.edit_plugins(|p| {
            p.shift_remove(&PROBE);
        });
        assert!(shared.graph.load().plugin(PROBE).is_none(), "{pin:?}: unpublished");
        sweep(&shared, 4);
        assert!(destroyed(&log).is_empty(), "{pin:?}: destroyed under a reader's pin");

        // The reader lets go. That release must not be the last reference.
        release.send(()).unwrap();
        let reader_thread = reader.join().expect("reader");
        assert!(
            destroyed(&log).is_empty(),
            "{pin:?}: the reader's unpin ran the destructor on the reader thread"
        );

        // The next sweeps (the graph, then the slot the edit retired on its
        // own) destroy it — here, on the engine thread.
        sweep(&shared, 2);
        let engine_thread = std::thread::current().id();
        assert_eq!(destroyed(&log), vec![engine_thread], "{pin:?}: destroyed once, by the sweep");
        assert_ne!(reader_thread, engine_thread);
        assert!(shared.retired.is_empty(), "{pin:?}: nothing left pinned");
    }
}

/// Replacing a slot under the same id retires the replaced one exactly
/// like a removal.
#[test]
fn a_slot_replaced_under_its_id_is_retired_like_a_removal() {
    let log = DestroyLog::default();
    let shared = SharedState::default();
    let first = Arc::new(probe_slot(&log));
    let weak = Arc::downgrade(&first);
    shared.edit_plugins(|p| p.insert(PROBE, first));
    let second = Arc::new(probe_slot(&log));
    shared.edit_plugins(|p| p.insert(PROBE, second));
    assert!(destroyed(&log).is_empty(), "never destroyed inside the edit");
    sweep(&shared, 2);
    assert_eq!(weak.strong_count(), 0);
    assert_eq!(destroyed(&log), vec![std::thread::current().id()]);
    assert_eq!(shared.plugins().len(), 1, "the replacement stays live");
}

/// The real handlers — every path that removes plugin instances (a slot
/// removal from a track, bus or master chain, a track removal taking its
/// chain, `ClearAll`) — unpublish without destroying inline, and the
/// engine loop's sweep destroys once the reader lets go.
#[test]
fn every_removal_handler_leaves_the_destroy_to_the_engine_sweep() {
    const TRACK: TrackId = 1;
    const BUS: BusId = 1;
    let cases: [(&str, fn(&mut EngineHandlerHarness)); 5] = [
        ("RemovePlugin", |h| {
            h.dispatch(AudioCommand::RemovePlugin {
                track_id: TRACK,
                instance_id: PROBE,
            })
        }),
        ("RemoveTrack", |h| h.remove_track(TRACK)),
        ("RemovePluginFromBus", |h| {
            h.dispatch(AudioCommand::RemovePluginFromBus {
                bus_id: BUS,
                instance_id: PROBE,
            })
        }),
        ("RemovePluginFromMaster", |h| {
            h.dispatch(AudioCommand::RemovePluginFromMaster { instance_id: PROBE })
        }),
        ("ClearAll", |h| h.clear_all()),
    ];
    for (name, remove) in cases {
        let log = DestroyLog::default();
        let mut h = EngineHandlerHarness::new();
        let track = Track::new(TRACK, "t".into());
        let _ = track.push_plugin(PROBE);
        h.push_track(track);
        h.add_bus(BUS, None);
        match name {
            "RemovePluginFromBus" => {
                h.shared()
                    .edit_bus(BUS, |b| b.plugin_ids.push(PROBE))
                    .expect("bus");
            }
            "RemovePluginFromMaster" => h.shared().edit_master(|m| m.plugin_ids.push(PROBE)),
            _ => {}
        }
        let slot = Arc::new(probe_slot(&log));
        h.shared().edit_plugins(|p| p.insert(PROBE, slot));
        assert_eq!(h.plugin_instance_count(), 1);

        let (pinned, release, reader) = spawn_reader(h.shared_arc(), Pin::Guard);
        pinned.recv().unwrap();
        remove(&mut h);
        assert_eq!(h.plugin_instance_count(), 0, "{name}: unpublished");
        h.sweep_retired();
        assert!(destroyed(&log).is_empty(), "{name}: destroyed inline or under a pin");

        release.send(()).unwrap();
        let reader_thread = reader.join().expect("reader");
        assert!(destroyed(&log).is_empty(), "{name}: destroyed by the reader");
        h.sweep_retired();
        h.sweep_retired();
        let engine_thread = std::thread::current().id();
        assert_eq!(destroyed(&log), vec![engine_thread], "{name}: destroyed by the sweep");
        assert_ne!(reader_thread, engine_thread);
    }
}

// ---------------------------------------------------------------------------
// Plugin edits under render
// ---------------------------------------------------------------------------

/// Two inaudible inserts on the audible track, and a scratch slot the
/// editor keeps adding, moving and removing.
const FX_A: PluginInstanceId = 10;
const FX_B: PluginInstanceId = 11;
const SCRATCH: PluginInstanceId = 50;

/// A minute of a constant 0.1 on `track_id`, from the top.
fn audio_clip(id: ClipId, track_id: TrackId) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![0.1; 2 * SR as usize * 60]),
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

/// A playing project: one audio track on a non-unity fader and pan, its
/// chain `[FX_A, FX_B]` of pass-through probes.
fn plugin_harness(log: &DestroyLog) -> MixAudioHarness {
    let track = Track::new(1, "t".into());
    track.set_volume(0.7);
    track.set_pan(-0.3);
    let _ = track.push_plugin(FX_A);
    let _ = track.push_plugin(FX_B);
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![audio_clip(1, 1)],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    let (a, b) = (Arc::new(probe_slot(log)), Arc::new(probe_slot(log)));
    h.edit_plugins(|p| {
        p.insert(FX_A, a);
        p.insert(FX_B, b);
    });
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

fn render_blocks(h: &mut MixAudioHarness) -> Vec<f32> {
    let mut out = Vec::with_capacity(BLOCKS * BLOCK * 2);
    for block in 0..BLOCKS {
        let data = h.render();
        assert!(data.iter().any(|&s| s != 0.0), "block {block} rendered");
        out.extend_from_slice(data);
    }
    out
}

/// What a hammer run observed.
struct Hammered {
    audio: Vec<f32>,
    edits: u64,
    adds: u64,
    editor_thread: ThreadId,
}

/// Render [`BLOCKS`] blocks while an editor thread publishes plugin edits
/// as fast as it can, in the handlers' order: add (slot published, then
/// named on the chain), reorder, remove (chain, then slot), a no-op
/// plugin-map publish — and, with `bypass`, a live per-slot bypass toggle
/// (`set_bypassed`, what `apply_bypass_request` does while rendering).
/// The editor sweeps the retire queue as the engine loop does, then winds
/// down: once the render loop has returned nothing pins a graph, so its
/// last sweeps free everything the edits retired.
///
/// Asserts what every run must hold: no skipped block, no lock miss, the
/// queue drained, and the drop discipline under load — every scratch
/// instance destroyed exactly once, by the editor's sweeps (the engine
/// loop), never by the render thread.
fn hammer(log: &DestroyLog, bypass: bool) -> Hammered {
    let mut h = plugin_harness(log);
    let shared = h.shared_arc();
    let stop = Arc::new(AtomicBool::new(false));
    let editor = {
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        let log = Arc::clone(log);
        std::thread::spawn(move || {
            let track = || Arc::clone(shared.tracks().get(&1).expect("track 1"));
            let remove_scratch = || {
                shared.retired.retire(track().retain_plugins(|&id| id != SCRATCH));
                shared.edit_plugins(|p| {
                    p.shift_remove(&SCRATCH);
                });
            };
            let mut edits = 0u64;
            let mut adds = 0u64;
            while !stop.load(Ordering::Acquire) {
                match edits % 5 {
                    0 if shared.graph.load().plugin(SCRATCH).is_none() => {
                        let slot = Arc::new(probe_slot(&log));
                        shared.edit_plugins(|p| p.insert(SCRATCH, slot));
                        shared.retired.retire(track().push_plugin(SCRATCH));
                        adds += 1;
                    }
                    1 => {
                        let to = (edits / 5 % 3) as usize;
                        let moved = if edits % 2 == 0 { SCRATCH } else { FX_B };
                        let _ = track().move_plugin_into(moved, to, |old| {
                            shared.retired.retire(old)
                        });
                    }
                    2 if bypass => {
                        let graph = shared.graph.load();
                        let fade = &graph.plugin(FX_A).expect("fx a").bypass;
                        fade.set_bypassed(!fade.bypassed());
                    }
                    3 => remove_scratch(),
                    4 => shared.edit_plugins(|_| {}),
                    _ => {}
                }
                edits += 1;
                if edits.is_multiple_of(64) {
                    // The engine loop's 16 ms tick.
                    shared.retired.sweep();
                }
            }
            remove_scratch();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !shared.retired.is_empty() && Instant::now() < deadline {
                shared.retired.sweep();
            }
            (edits, adds, std::thread::current().id())
        })
    };

    let audio = render_blocks(&mut h);
    stop.store(true, Ordering::Release);
    let (edits, adds, editor_thread) = editor.join().expect("editor");

    assert!(adds > 0, "the editor actually added and removed plugins ({edits} edits)");
    assert_eq!(
        h.shared().render_skip_cycles.load(Ordering::Relaxed),
        0,
        "no block was skipped across {edits} concurrent plugin edits"
    );
    assert_eq!(h.shared().lock_misses.snapshot(), [0; STATE_MAP_COUNT]);
    assert!(h.shared().retired.is_empty(), "the editor's sweeps freed everything");
    let destroys = destroyed(log);
    assert_eq!(destroys.len() as u64, adds, "every scratch instance destroyed, once");
    assert!(
        destroys.iter().all(|&t| t == editor_thread),
        "a plugin was destroyed off the engine (sweeping) thread"
    );
    assert!(!destroys.contains(&std::thread::current().id()), "never by the render thread");
    Hammered {
        audio,
        edits,
        adds,
        editor_thread,
    }
}

/// Structural plugin edits under render. Every insert is a pass-through,
/// so the output must be bit-identical to an unedited run.
#[test]
fn plugin_edits_published_while_the_callback_renders_never_skip_a_block() {
    let reference = render_blocks(&mut plugin_harness(&DestroyLog::default()));
    let run = hammer(&DestroyLog::default(), false);
    assert!(
        run.audio == reference,
        "{} edits ({} adds) changed the rendered audio (first diff at sample {:?})",
        run.edits,
        run.adds,
        run.audio.iter().zip(&reference).position(|(a, b)| a != b)
    );
    assert_ne!(run.editor_thread, std::thread::current().id());
}

/// The same with live per-slot bypass toggles mixed in. A toggle
/// crossfades the slot against its dry copy (equal-gain, so across a
/// pass-through it is transparent up to float rounding) — close to the
/// reference, not bit-identical. The point is the shared assertions:
/// bypass reaches the slot through the graph without a lock, and no
/// block is skipped.
#[test]
fn plugin_edits_and_bypass_toggles_under_render_never_skip_a_block() {
    let reference = render_blocks(&mut plugin_harness(&DestroyLog::default()));
    let run = hammer(&DestroyLog::default(), true);
    let worst = run
        .audio
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst <= 1e-6, "a bypass fade over a pass-through moved a sample by {worst}");
}

