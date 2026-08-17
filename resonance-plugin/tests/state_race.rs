//! Race a main-thread `state::load` against an in-flight audio-thread
//! `process()`.
//!
//! CLAP allows `clap_plugin_state.load` ([main-thread]) to run while
//! `clap_plugin.process` ([audio-thread]) is executing. The bridge handles
//! that with per-param atomics, a `params_dirty` flag, and a
//! compare-exchange in the editor push-back loop at the end of
//! `clap_bridge/process.rs`. Until now none of it had a concurrency test.
//!
//! # Why these tests are deterministic and still genuinely concurrent
//!
//! Letting two free-running threads collide and hoping to land in a
//! nanosecond-wide window gives a test that is flaky in one direction and
//! vacuous in the other. Instead, one of the plugin's parameters is a
//! `HookParam` whose `get_plain()` **suspends the audio thread** at a chosen
//! point and signals the main thread. The main thread then performs the real
//! `state::load` while `process()` is genuinely mid-block on another OS
//! thread, and releases it.
//!
//! Because the bridge's push-back loop walks parameters in slot order, the
//! slot the hook occupies decides which side of the `loaded` parameter's
//! read the load lands on:
//!
//! ```text
//!   slot 0  hook_early   <- load here: BEFORE `loaded` is read back
//!   slot 1  loaded       <- written only by state::load
//!   slot 2  hook_late    <- load here: AFTER `loaded` is read back
//!   slot 3  edited       <- written only by the "editor" thread
//! ```
//!
//! That gives full control of the interleaving with no sleeps, no retries
//! and no serialisation of the two operations.
//!
//! ba todo #1341.

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use clack_extensions::params::PluginParams;
use clack_extensions::state::PluginState;
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use common::{TestHost, TestHostShared};
use resonance_plugin::{
    stable_hash, ClapBridge, EventIterator, FloatParam, FloatRange, OutputBuffer, Param,
    ResonancePlugin, TempoInfo,
};
use serde_json::{json, Value};

/// No test here should ever block for this long; the timeouts exist so a
/// regression reports a failure instead of wedging the whole suite.
const PATIENCE: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// A gate that can suspend the audio thread at a chosen point inside process()
// ---------------------------------------------------------------------------

#[derive(Default)]
struct GateState {
    armed: bool,
    reached: bool,
    released: bool,
}

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    cv: Condvar,
}

impl Gate {
    /// Main thread: stop the audio thread the next time it passes this point.
    fn arm(&self) {
        let mut state = self.state.lock().unwrap();
        *state = GateState {
            armed: true,
            reached: false,
            released: false,
        };
        drop(state);
    }

    /// Audio thread, from inside `process()`.
    fn checkpoint(&self) {
        let mut state = self.state.lock().unwrap();
        if !state.armed {
            return;
        }
        state.armed = false;
        state.reached = true;
        self.cv.notify_all();

        let deadline = Instant::now() + PATIENCE;
        while !state.released {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Never wedge the suite: give up and let the block finish.
                break;
            }
            state = self.cv.wait_timeout(state, remaining).unwrap().0;
        }
    }

    /// Main thread: block until `process()` is suspended at this point.
    fn wait_until_reached(&self) {
        let mut state = self.state.lock().unwrap();
        let deadline = Instant::now() + PATIENCE;
        while !state.reached {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "the audio thread never reached the gate"
            );
            state = self.cv.wait_timeout(state, remaining).unwrap().0;
        }
    }

    /// Main thread: let the suspended block continue.
    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.cv.notify_all();
    }
}

/// A parameter that behaves exactly like a `FloatParam`, except that reading
/// it can suspend the caller. The bridge reads every parameter with
/// `get_plain()` in its push-back loop, which is what makes this a precise
/// scalpel for placing a concurrent main-thread write.
struct HookParam {
    inner: FloatParam,
    gate: Gate,
}

impl HookParam {
    fn new(id: &'static str) -> Self {
        Self {
            inner: FloatParam::new(id, id, 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            gate: Gate::default(),
        }
    }
}

impl Param for HookParam {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn get_plain(&self) -> f64 {
        self.gate.checkpoint();
        self.inner.get_plain()
    }
    fn set_plain(&self, v: f64) {
        self.inner.set_plain(v);
    }
    fn default_plain(&self) -> f64 {
        self.inner.default_plain()
    }
    fn min_plain(&self) -> f64 {
        self.inner.min_plain()
    }
    fn max_plain(&self) -> f64 {
        self.inner.max_plain()
    }
    fn display(&self, value: f64) -> String {
        self.inner.display(value)
    }
    fn parse(&self, text: &str) -> Option<f64> {
        self.inner.parse(text)
    }
}

// ---------------------------------------------------------------------------
// The plugin under test
// ---------------------------------------------------------------------------

const LOADED_MAX: f32 = 1000.0;

struct RaceParams {
    hook_early: HookParam,
    loaded: FloatParam,
    hook_late: HookParam,
    edited: FloatParam,
}

impl RaceParams {
    fn new() -> Self {
        Self {
            hook_early: HookParam::new("hook_early"),
            loaded: FloatParam::new(
                "loaded",
                "Loaded",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: LOADED_MAX,
                },
            ),
            hook_late: HookParam::new("hook_late"),
            edited: FloatParam::new(
                "edited",
                "Edited",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: LOADED_MAX,
                },
            ),
        }
    }

    fn as_slice(&self) -> [&dyn Param; 4] {
        [
            &self.hook_early,
            &self.loaded,
            &self.hook_late,
            &self.edited,
        ]
    }
}

/// Each test gets its own plugin type — and so its own `OnceLock` of shared
/// parameters — because cargo runs the tests in this binary concurrently and
/// they would otherwise fight over one plugin's state.
macro_rules! race_plugin {
    ($ty:ident, $slot:ident, $params:ident, $id:literal) => {
        static $slot: OnceLock<Arc<RaceParams>> = OnceLock::new();

        /// The handle a real plugin hands to its editor: the very same param
        /// storage the audio thread is reading.
        fn $params() -> Arc<RaceParams> {
            $slot.get_or_init(|| Arc::new(RaceParams::new())).clone()
        }

        struct $ty {
            params: Arc<RaceParams>,
        }

        impl ResonancePlugin for $ty {
            const CLAP_ID: &'static str = $id;
            const NAME: &'static str = $id;
            const VENDOR: &'static str = "test";
            const VERSION: &'static str = "0.0.0";
            const DESCRIPTION: &'static str = "";
            const FEATURES: &'static [&'static std::ffi::CStr] =
            &[resonance_plugin::features::AUDIO_EFFECT];
            const INPUT_CHANNELS: Option<u32> = None;

            fn new() -> Self {
                Self { params: $params() }
            }
            fn param_count(&self) -> usize {
                4
            }
            fn param(&self, index: usize) -> &dyn Param {
                self.params.as_slice()[index]
            }
            fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
                true
            }
            fn reset(&mut self) {}
            fn process(
                &mut self,
                _outputs: &mut [OutputBuffer<'_>],
                _frames: usize,
                _events: &mut EventIterator<'_>,
                _tempo: Option<TempoInfo>,
            ) {
            }
        }
    };
}

race_plugin!(EditorPlugin, EDITOR_SLOT, editor_params, "test.race-editor");
race_plugin!(LatePlugin, LATE_SLOT, late_params, "test.race-late");
race_plugin!(EarlyPlugin, EARLY_SLOT, early_params, "test.race-early");

// ---------------------------------------------------------------------------
// Host side: an instance on this thread, its audio processor on another
// ---------------------------------------------------------------------------

const FRAMES: usize = 64;

fn activate<P: ResonancePlugin>(
    plugin_id: &std::ffi::CStr,
) -> (
    PluginInstance<TestHost>,
    StartedPluginAudioProcessor<TestHost>,
) {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<P>>>(c"race.clap")
        .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    let mut instance =
        PluginInstance::<TestHost>::new(|_| TestHostShared, |_| (), &entry, plugin_id, &host_info)
            .expect("plugin instantiation");

    let processor = instance
        .activate(
            |_, _| (),
            PluginAudioConfiguration {
                sample_rate: 48_000.0,
                min_frames_count: 32,
                max_frames_count: 8192,
            },
        )
        .expect("activation")
        .start_processing()
        .expect("start processing");

    (instance, processor)
}

/// A real audio thread: its own OS thread calling `process()` back to back,
/// exactly as a host's callback would, for as long as the test needs.
struct AudioThread {
    stop: Arc<AtomicBool>,
    blocks: Arc<AtomicU64>,
    handle: Option<JoinHandle<StartedPluginAudioProcessor<TestHost>>>,
}

fn spawn_audio(mut processor: StartedPluginAudioProcessor<TestHost>) -> AudioThread {
    let stop = Arc::new(AtomicBool::new(false));
    let blocks = Arc::new(AtomicU64::new(0));

    let handle = {
        let stop = Arc::clone(&stop);
        let blocks = Arc::clone(&blocks);
        thread::spawn(move || {
            let mut ports = AudioPorts::with_capacity(2, 1);
            let mut buffers = [[0.0_f32; FRAMES]; 2];
            while !stop.load(Ordering::Relaxed) {
                let mut outputs = ports.with_output_buffers([AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_output_only(
                        buffers.iter_mut().map(|b| b.as_mut_slice()),
                    ),
                }]);
                let mut output_events = EventBuffer::new();
                processor
                    .process(
                        &InputAudioBuffers::empty(),
                        &mut outputs,
                        &InputEvents::empty(),
                        &mut output_events.as_output(),
                        None,
                        None,
                    )
                    .expect("process");
                blocks.fetch_add(1, Ordering::Release);
            }
            processor
        })
    };

    AudioThread {
        stop,
        blocks,
        handle: Some(handle),
    }
}

impl AudioThread {
    /// Block until at least `n` further blocks have run to completion.
    fn wait_blocks(&self, n: u64) {
        let target = self.blocks.load(Ordering::Acquire) + n;
        let deadline = Instant::now() + PATIENCE;
        while self.blocks.load(Ordering::Acquire) < target {
            assert!(Instant::now() < deadline, "the audio thread stalled");
            thread::yield_now();
        }
    }

    fn stop(mut self) -> StartedPluginAudioProcessor<TestHost> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .expect("audio thread handle")
            .join()
            .expect("audio thread panicked")
    }
}

// ---------------------------------------------------------------------------
// Host-side helpers
// ---------------------------------------------------------------------------

fn clap_id(id: &str) -> ClapId {
    ClapId::new(stable_hash(id))
}

fn get_value(instance: &mut PluginInstance<TestHost>, id: &str) -> f64 {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params extension")
        .get_value(&mut instance.plugin_handle(), clap_id(id))
        .unwrap_or_else(|| panic!("param `{id}` is unknown to the bridge"))
}

/// A preset that sets **only** `loaded`, so nothing else in the plugin is
/// disturbed while the test is watching one slot.
fn state_bytes(loaded: f64) -> Vec<u8> {
    serde_json::to_vec(&json!({ "params": { "loaded": loaded } })).expect("state json")
}

fn load_state(instance: &mut PluginInstance<TestHost>, bytes: &[u8]) {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("state extension")
        .load(&mut instance.plugin_handle(), &mut &bytes[..])
        .expect("state load");
}

fn save_state(instance: &mut PluginInstance<TestHost>) -> Value {
    let mut bytes = Vec::new();
    instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("state extension")
        .save(&mut instance.plugin_handle(), &mut bytes)
        .expect("state save");
    serde_json::from_slice(&bytes).expect("the bridge must save valid JSON")
}

fn shutdown(mut instance: PluginInstance<TestHost>, audio: AudioThread) {
    let processor = audio.stop();
    instance.deactivate(processor.stop_processing());
}

// ---------------------------------------------------------------------------
// No lost editor write
// ---------------------------------------------------------------------------

#[test]
fn an_editor_write_racing_the_audio_thread_reaches_the_host() {
    let params = editor_params();
    let (mut instance, processor) = activate::<EditorPlugin>(c"test.race-editor");
    let audio = spawn_audio(processor);

    // The "editor": a thread writing straight into the plugin's own atomic
    // param storage through the `Arc` a real editor is handed — no CLAP
    // event, no main-thread involvement. This is the path the push-back
    // loop in `process()` exists to rescue.
    let stop = Arc::new(AtomicBool::new(false));
    let editor = {
        let params = Arc::clone(&params);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut value = 0.0_f64;
            while !stop.load(Ordering::Relaxed) {
                value = if value >= 900.0 { 1.0 } else { value + 1.0 };
                params.edited.set_plain(value);
            }
            value
        })
    };

    // Let the two threads genuinely overlap for a while.
    audio.wait_blocks(500);
    stop.store(true, Ordering::Relaxed);
    let last_written = editor.join().expect("editor thread panicked");

    // A couple of blocks is all the push-back needs to carry the final write
    // into the shared atomics.
    audio.wait_blocks(3);

    assert_eq!(
        get_value(&mut instance, "edited"),
        last_written,
        "the host must read back the value the editor last wrote, not a \
         stale one the audio thread overwrote"
    );

    // The same value has to survive a project save, which is served from the
    // shared atomics while the plugin is active. Without the push-back this
    // is where a user's editor edits silently became defaults again.
    assert_eq!(
        save_state(&mut instance)["params"]["edited"],
        json!(last_written),
        "an active-plugin save must persist the editor's value"
    );

    shutdown(instance, audio);
}

// ---------------------------------------------------------------------------
// No lost state load
// ---------------------------------------------------------------------------

#[test]
fn a_state_load_landing_after_the_push_back_read_survives() {
    let params = late_params();
    let (mut instance, processor) = activate::<LatePlugin>(c"test.race-late");
    let audio = spawn_audio(processor);

    for round in 1..=8_u32 {
        let target = f64::from(round) * 10.0;

        params.hook_late.gate.arm();
        params.hook_late.gate.wait_until_reached();

        // `process()` is suspended right now, part-way through its push-back
        // loop, on the audio thread — past the `loaded` slot. The load below
        // runs concurrently with that in-flight block.
        load_state(&mut instance, &state_bytes(target));
        params.hook_late.gate.release();

        audio.wait_blocks(3);

        assert_eq!(
            get_value(&mut instance, "loaded"),
            target,
            "round {round}: the loaded value must survive in the shared atomics"
        );
        assert_eq!(
            params.loaded.get_plain(),
            target,
            "round {round}: `params_dirty` must carry the loaded value into \
             the plugin's own storage on a later block"
        );
    }

    shutdown(instance, audio);
}

/// **This test pins a defect, not a guarantee.**
///
/// When the load lands after `process()` has already swapped `params_dirty`
/// but before the push-back loop reads the slot back, the compare-exchange
/// does not protect it: the loop reads the *freshly loaded* value as its
/// `current`, so the exchange **succeeds** and stores the plugin's stale
/// value over it. `params_dirty` is still set, so the next block copies that
/// stale value back into the plugin, and the load is lost permanently.
///
/// The comment on the CAS in `clap_bridge/process.rs` claims "a concurrent
/// main-thread write makes the exchange fail". That only holds for writes
/// landing between the `get_value` read and the exchange itself — a window a
/// few instructions wide. The wider window, from the `params_dirty` swap to
/// that read, is unprotected, and this test reaches it deterministically.
///
/// Filed as ba todo #1363. When that lands, this test flips to asserting
/// `target` and is renamed to match `..._survives` above.
#[test]
fn a_state_load_landing_before_the_push_back_read_is_currently_lost() {
    let params = early_params();
    let (mut instance, processor) = activate::<EarlyPlugin>(c"test.race-early");
    let audio = spawn_audio(processor);

    let before = params.loaded.get_plain();
    let target = 250.0_f64;
    assert_ne!(before, target, "the test must actually change something");

    params.hook_early.gate.arm();
    params.hook_early.gate.wait_until_reached();

    // `process()` is suspended in its push-back loop *before* the `loaded`
    // slot, and has already taken `params_dirty`.
    load_state(&mut instance, &state_bytes(target));
    params.hook_early.gate.release();

    audio.wait_blocks(3);

    assert_eq!(
        get_value(&mut instance, "loaded"),
        before,
        "documenting today's behaviour: the push-back clobbers the load. If \
         this now reads {target}, the defect is fixed — flip the assertion."
    );
    assert_eq!(
        params.loaded.get_plain(),
        before,
        "and the stale value is copied back into the plugin on the next block"
    );

    shutdown(instance, audio);
}
