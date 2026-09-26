//! A real multi-output instrument, rendered through the real stem path.
//!
//! Extracted from `stem_bus_sub_track_render.rs` (ba todo #1239) so the
//! Track-arm tests (ba todo #1242) can share it instead of copying 250
//! lines of `clap_sys` vtable boilerplate.
//!
//! Sub-track audio cannot be produced with plain clips — `render_core`
//! skips sub-tracks in the clip pass, because their signal arrives only
//! from the parent instrument's port fan-out. So this module hosts a
//! hand-rolled **multi-output** CLAP plugin built straight from
//! `clap_sys` vtables (no shared library) via the
//! `__instance_from_raw_for_test` hook, the same technique
//! `clap_latency_tracking.rs` uses for the latency machinery. It declares
//! three stereo output ports and writes a distinct constant per port, so
//! every level in the assertions built on it is exactly known.
//!
//! The per-port constants are **per instance** (ba todo #1242): the
//! default set leaves port 0 silent, which is the drum kit's real
//! behaviour (ba doc #274 §1a), but a test that needs to see whether the
//! parent's own main output leaks into a stem has to be able to make
//! port 0 audible. A global would race — the tests in one binary run in
//! parallel threads — so the levels live in the plugin's `plugin_data`.

#![allow(dead_code)]

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::Arc;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use indexmap::IndexMap;
use parking_lot::RwLock;

use resonance_audio::test_support::{
    render_stem, PluginMap, PluginSlot, SharedState, StemSource, __instance_from_raw_for_test,
};
use resonance_audio::types::*;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

pub const SR: u32 = 48_000;
/// Output ports the fake instrument declares: port 0 ("main") plus two
/// group taps, mirroring the shape of `com.resonance.drums`.
pub const PORTS: usize = 3;
/// Default constant each port writes: `PORT_LEVELS[i]` on both channels.
/// Port 0 is silent on purpose — that is the drum kit's real behaviour
/// (doc #274 §1a) and it is what makes a bus stem's level unambiguous.
pub const PORT_LEVELS: [f32; PORTS] = [0.0, 0.25, 0.125];

pub const FRAMES: u64 = 512;
pub const INSTRUMENT_ID: PluginInstanceId = 100;

/// Track ids the harness lays out: one parent instrument track and one
/// sub-track per extra output port.
pub const PARENT: TrackId = 1;
pub const TAP_A: TrackId = 10;
pub const TAP_B: TrackId = 11;
/// A SECOND multi-output instrument, added on demand by
/// [`EngineState::add_unfrozen_sibling`] (ba todo #1248).
pub const SIBLING: TrackId = 2;
pub const SIBLING_TAP: TrackId = 20;

// ---------------------------------------------------------------------------
// Fake multi-output CLAP instrument
// ---------------------------------------------------------------------------

struct FakeState {
    active: bool,
    levels: [f32; PORTS],
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    &mut *((*plugin).plugin_data as *mut FakeState)
}

unsafe extern "C" fn fake_init(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_destroy(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_activate(
    plugin: *const clap_plugin,
    _sample_rate: f64,
    _min_frames: u32,
    _max_frames: u32,
) -> bool {
    fake_state(plugin).active = true;
    true
}

unsafe extern "C" fn fake_deactivate(plugin: *const clap_plugin) {
    fake_state(plugin).active = false;
}

unsafe extern "C" fn fake_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_stop_processing(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_reset(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_on_main_thread(_plugin: *const clap_plugin) {}

/// Write this instance's `levels[i]` into every frame of output port `i`.
/// Ports the host did not hand us are skipped, and any port the host
/// provides past our declared count is left untouched.
unsafe extern "C" fn fake_process(plugin: *const clap_plugin, process: *const clap_process) -> i32 {
    let levels = fake_state(plugin).levels;
    let p = &*process;
    let frames = p.frames_count as usize;
    let out_count = (p.audio_outputs_count as usize).min(PORTS);
    for (port, &level) in levels.iter().enumerate().take(out_count) {
        let buf: &clap_audio_buffer = &*p.audio_outputs.add(port);
        if buf.data32.is_null() {
            continue;
        }
        for ch in 0..(buf.channel_count as usize).min(2) {
            let chan = *buf.data32.add(ch);
            if chan.is_null() {
                continue;
            }
            for f in 0..frames {
                *chan.add(f) = level;
            }
        }
    }
    // CLAP_PROCESS_CONTINUE
    1
}

unsafe extern "C" fn fake_ports_count(_plugin: *const clap_plugin, is_input: bool) -> u32 {
    if is_input {
        1
    } else {
        PORTS as u32
    }
}

unsafe extern "C" fn fake_ports_get(
    _plugin: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    let limit = if is_input { 1 } else { PORTS as u32 };
    if index >= limit || info.is_null() {
        return false;
    }
    let out = &mut *info;
    out.id = index as clap_id;
    out.name = [0; 256];
    let label = match index {
        0 => b"Main\0".as_ref(),
        1 => b"Tap A\0".as_ref(),
        _ => b"Tap B\0".as_ref(),
    };
    for (slot, byte) in out.name.iter_mut().zip(label.iter()) {
        *slot = *byte as c_char;
    }
    out.flags = 0;
    out.channel_count = 2;
    out.port_type = ptr::null();
    out.in_place_pair = u32::MAX;
    true
}

static FAKE_AUDIO_PORTS_EXT: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(fake_ports_count),
    get: Some(fake_ports_get),
};

unsafe extern "C" fn fake_get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    if CStr::from_ptr(id) == CStr::from_ptr(CLAP_EXT_AUDIO_PORTS.as_ptr()) {
        return &FAKE_AUDIO_PORTS_EXT as *const clap_plugin_audio_ports as *const c_void;
    }
    ptr::null()
}

/// Build the fake instrument and run it through the host's real
/// create/init/activate/start sequence.
pub fn multi_out_instrument(levels: [f32; PORTS]) -> PluginSlot {
    let inst = __instance_from_raw_for_test(
        move |_host| {
            let state = Box::into_raw(Box::new(FakeState {
                active: false,
                levels,
            }));
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(fake_init),
                destroy: Some(fake_destroy),
                activate: Some(fake_activate),
                deactivate: Some(fake_deactivate),
                start_processing: Some(fake_start_processing),
                stop_processing: Some(fake_stop_processing),
                reset: Some(fake_reset),
                process: Some(fake_process),
                get_extension: Some(fake_get_extension),
                on_main_thread: Some(fake_on_main_thread),
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        SR,
    )
    .expect("fake multi-output instrument builds");
    assert_eq!(
        inst.output_port_count(),
        PORTS,
        "the host must see all three declared output ports"
    );
    PluginSlot::new(inst)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

pub struct EngineState {
    pub shared: Arc<SharedState>,
    pub tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    pub busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    pub master: Arc<RwLock<MasterBus>>,
    pub clips: Arc<RwLock<Vec<AudioClip>>>,
    pub plugins: Arc<RwLock<PluginMap>>,
    pub tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

impl EngineState {
    /// One instrument track ([`PARENT`]) hosting the three-port fake,
    /// with a sub-track per extra port ([`TAP_A`] / [`TAP_B`]), every
    /// track routed to master.
    pub fn new() -> Self {
        Self::with_port_levels(PORT_LEVELS)
    }

    /// As [`EngineState::new`], with the instrument's per-port constants
    /// chosen by the caller — used to make port 0 audible (ba todo
    /// #1242).
    pub fn with_port_levels(levels: [f32; PORTS]) -> Self {
        let state = Self {
            shared: Arc::new(SharedState::default()),
            tracks: Arc::new(RwLock::new(IndexMap::new())),
            busses: Arc::new(RwLock::new(IndexMap::new())),
            master: Arc::new(RwLock::new(MasterBus::new())),
            clips: Arc::new(RwLock::new(Vec::new())),
            plugins: Arc::new(RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
        };
        state
            .plugins
            .write()
            .insert(INSTRUMENT_ID, multi_out_instrument(levels));

        let parent = Track::with_type(PARENT, "Kit".into(), TrackType::Instrument);
        parent.set_output(TrackOutput::Master);
        parent.push_plugin(INSTRUMENT_ID);
        state.tracks.write().insert(PARENT, parent);
        for (id, port) in [(TAP_A, 1u32), (TAP_B, 2)] {
            let sub = Track::new_sub_track(id, format!("Tap {port}"), PARENT, port);
            sub.set_output(TrackOutput::Master);
            state.tracks.write().insert(id, sub);
        }
        state
    }

    pub fn set_output(&self, id: TrackId, output: TrackOutput) {
        self.tracks.read().get(&id).unwrap().set_output(output);
    }

    /// Attach a freeze cache to `id`, as `SetTrackFrozenSource` does
    /// after a successful freeze (ba todo #1248).
    ///
    /// `dc` is the constant the cache holds on both channels. For a
    /// multi-output parent that is the WHOLE baked fan-out — freeze
    /// captures with `freeze_raw`, which forces every sub-track into
    /// master at unity so the parent's single cache file carries the
    /// summed kit (`render_core`'s `force_master_route`). That is
    /// precisely what makes a sub-track stem of a frozen parent hard:
    /// the tap's own signal is no longer separable from its siblings'.
    pub fn freeze(&self, id: TrackId, dc: f32, frames: usize) {
        let samples = Arc::new(vec![dc; frames * 2]);
        let cache_ref = FreezeCacheRef::new(
            "frozen-kit.wav".into(),
            SR,
            32,
            1,
            FreezeCacheStatus::Frozen,
        );
        let source = FrozenSource::new(cache_ref, samples, SR, frames as u64);
        self.tracks
            .read()
            .get(&id)
            .unwrap()
            .frozen_source
            .store(Some(Arc::new(source)));
    }

    /// Drop `id`'s freeze cache, as an unfreeze does.
    pub fn unfreeze(&self, id: TrackId) {
        self.tracks.read().get(&id).unwrap().frozen_source.store(None);
    }

    /// Add a SECOND multi-output instrument with one tap, so a test can
    /// assert that freezing one instrument leaves another alone.
    pub fn add_unfrozen_sibling(&self, parent: TrackId, tap: TrackId) {
        let id = INSTRUMENT_ID + parent;
        self.plugins
            .write()
            .insert(id, multi_out_instrument(PORT_LEVELS));
        let track = Track::with_type(parent, "Other Kit".into(), TrackType::Instrument);
        track.set_output(TrackOutput::Master);
        track.push_plugin(id);
        self.tracks.write().insert(parent, track);
        let sub = Track::new_sub_track(tap, "Other Tap 1".into(), parent, 1);
        sub.set_output(TrackOutput::Master);
        self.tracks.write().insert(tap, sub);
    }

    pub fn add_bus(&self, id: BusId, name: &str) {
        self.busses.write().insert(id, Bus::new(id, name.into()));
    }

    pub fn render(&self, source: StemSource) -> Vec<f32> {
        self.try_render(source).expect("render succeeds")
    }

    /// As [`EngineState::render`], surfacing the engine's refusal instead
    /// of panicking on it (ba todo #1248).
    pub fn try_render(&self, source: StemSource) -> Result<Vec<f32>, String> {
        render_stem(
            source,
            0,
            FRAMES,
            &self.shared,
            &self.tracks,
            &self.busses,
            &self.master,
            &self.clips,
            &self.plugins,
            &self.tempo_map,
            SR,
        )
        .map_err(|e| e.to_string())
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}

/// What a centre-panned track's signal measures once it reaches master.
///
/// The mixer's pan control is a stereo BALANCE, so dead centre is unity
/// and this is the identity (ba doc #276 BUG 3 — it used to be the
/// constant-power `1/sqrt(2)`, which cost every centred track 3 dB and
/// made a stem placed back into the project measure 3 dB below its
/// source). Kept as a named function so level assertions still say which
/// stage they are measuring at, and so a future pan-law change has one
/// place to land.
pub fn at_master(level: f32) -> f32 {
    level
}

/// Peak absolute sample across an interleaved-stereo buffer.
pub fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}
