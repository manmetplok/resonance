//! Bus stems fed by a multi-output instrument's sub-tracks (ba todo #1239).
//!
//! `stem_filter` used to resolve a bus's membership by scanning only
//! top-level tracks, so a bus fed exclusively by an instrument's group
//! taps came out EMPTY — and its stem export, and every `meter.measure` /
//! `meter.stems` built on the same filter (ba todo #1218), reported
//! digital silence rather than an error. That routing is precisely what
//! ba doc #274 recommends for glue-compressing a multi-output instrument,
//! so the filter tests in `stem_render.rs` are not enough: this file
//! proves it end to end, through a real render.
//!
//! Sub-track audio cannot be produced with plain clips — `render_core`
//! skips sub-tracks in the clip pass, because their signal arrives only
//! from the parent instrument's port fan-out. So this file hosts a
//! hand-rolled **multi-output** CLAP plugin built straight from
//! `clap_sys` vtables (no shared library) via the
//! `__instance_from_raw_for_test` hook, the same technique
//! `clap_latency_tracking.rs` uses for the latency machinery. It declares
//! three stereo output ports and writes a distinct constant per port, so
//! every level in the assertions below is exactly known.

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
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{
    __instance_from_raw_for_test, render_stem, stem_filter, SharedState, StemSource,
    SyncClapInstance,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
/// Output ports the fake instrument declares: port 0 ("main") plus two
/// group taps, mirroring the shape of `com.resonance.drums`.
const PORTS: usize = 3;
/// Constant each port writes: `PORT_LEVELS[i]` on both channels.
/// Port 0 is silent on purpose — that is the drum kit's real behaviour
/// (doc #274 §1a) and it is what makes the bus stem's level unambiguous.
const PORT_LEVELS: [f32; PORTS] = [0.0, 0.25, 0.125];

// ---------------------------------------------------------------------------
// Fake multi-output CLAP instrument
// ---------------------------------------------------------------------------

struct FakeState {
    active: bool,
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

/// Write `PORT_LEVELS[i]` into every frame of output port `i`. Ports the
/// host did not hand us are skipped, and any port the host provides past
/// our declared count is left untouched.
unsafe extern "C" fn fake_process(
    _plugin: *const clap_plugin,
    process: *const clap_process,
) -> i32 {
    let p = &*process;
    let frames = p.frames_count as usize;
    let out_count = (p.audio_outputs_count as usize).min(PORTS);
    for (port, &level) in PORT_LEVELS.iter().enumerate().take(out_count) {
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
fn multi_out_instrument() -> SyncClapInstance {
    let inst = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(FakeState { active: false }));
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
    SyncClapInstance(inst)
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct EngineState {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

const FRAMES: u64 = 512;
const INSTRUMENT_ID: PluginInstanceId = 100;

impl EngineState {
    /// One instrument track (id 1) hosting the three-port fake, with a
    /// sub-track per extra port (ids 10 and 11), plus a MIDI clip that
    /// gives the range something to render over.
    fn new() -> Self {
        let state = Self {
            shared: Arc::new(SharedState::default()),
            tracks: Arc::new(RwLock::new(IndexMap::new())),
            busses: Arc::new(RwLock::new(IndexMap::new())),
            master: Arc::new(RwLock::new(MasterBus::new())),
            clips: Arc::new(RwLock::new(Vec::new())),
            midi_clips: Arc::new(RwLock::new(Vec::new())),
            plugins: Arc::new(RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
        };
        state
            .plugins
            .write()
            .insert(INSTRUMENT_ID, Mutex::new(multi_out_instrument()));

        let parent = Track::with_type(1, "Kit".into(), TrackType::Instrument);
        parent.set_output(TrackOutput::Master);
        parent.push_plugin(INSTRUMENT_ID);
        state.tracks.write().insert(1, parent);
        for (id, port) in [(10u64, 1u32), (11, 2)] {
            let sub = Track::new_sub_track(id, format!("Tap {port}"), 1, port);
            sub.set_output(TrackOutput::Master);
            state.tracks.write().insert(id, sub);
        }
        state
    }

    fn set_output(&self, id: TrackId, output: TrackOutput) {
        self.tracks.read().get(&id).unwrap().set_output(output);
    }

    fn add_bus(&self, id: BusId, name: &str) {
        self.busses.write().insert(id, Bus::new(id, name.into()));
    }

    fn render(&self, source: StemSource) -> Vec<f32> {
        render_stem(
            source,
            0,
            FRAMES,
            &self.shared,
            &self.tracks,
            &self.busses,
            &self.master,
            &self.clips,
            &self.midi_clips,
            &self.plugins,
            &self.tempo_map,
            SR,
        )
        .expect("render succeeds")
    }
}

/// Peak absolute sample across an interleaved-stereo buffer.
fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The bug, end to end: an instrument's group taps routed into a bus of
/// their own must render as that bus's stem. Before ba todo #1239 the
/// filter was empty and this buffer was digital silence.
#[test]
fn bus_stem_fed_only_by_sub_tracks_is_not_silent() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(10, TrackOutput::Bus(7));
    state.set_output(11, TrackOutput::Bus(7));

    let filter = stem_filter(StemSource::Bus(7), &state.tracks.read());
    assert!(
        filter.contains(10) && filter.contains(11),
        "both taps feed the bus: {:?}",
        filter.set
    );

    let stem = state.render(StemSource::Bus(7));
    assert_eq!(stem.len(), FRAMES as usize * 2);
    let bus_peak = peak(&stem);
    assert!(
        bus_peak > 0.05,
        "the bus stem must carry the taps' audio, got peak {bus_peak}"
    );

    // The taps are the ONLY contributors, so the bus stem must equal the
    // parent's whole-instrument stem (port 0 is silent by construction,
    // as it is on the real drum kit).
    let track_stem = state.render(StemSource::Track(1));
    let track_peak = peak(&track_stem);
    assert!(
        (bus_peak - track_peak).abs() < 1e-6,
        "bus stem ({bus_peak}) must carry exactly the instrument's audio ({track_peak})"
    );
}

/// The pre-existing path must not regress: a top-level track routed to a
/// bus still brings its sub-tracks into that bus's stem.
#[test]
fn bus_stem_still_includes_a_routed_parents_sub_tracks() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(1, TrackOutput::Bus(7));

    let filter = stem_filter(StemSource::Bus(7), &state.tracks.read());
    assert!(filter.contains(1), "the routed parent");
    assert!(
        filter.contains(10) && filter.contains(11),
        "its sub-tracks ride along: {:?}",
        filter.set
    );

    let stem = state.render(StemSource::Bus(7));
    assert!(
        peak(&stem) > 0.05,
        "the routed instrument still reaches its bus stem"
    );
}

/// A parent and one of its sub-tracks both pointed at the same bus must
/// contribute their audio ONCE. `stem_filter` dedupes the ids; this pins
/// the render side, which is what a set alone cannot prove.
#[test]
fn parent_and_sub_track_on_one_bus_are_not_summed_twice() {
    // Baseline: taps routed to the bus, parent on master.
    let baseline = {
        let state = EngineState::new();
        state.add_bus(7, "Kit Bus");
        state.set_output(10, TrackOutput::Bus(7));
        state.set_output(11, TrackOutput::Bus(7));
        state.render(StemSource::Bus(7))
    };

    // Same routing, but the (silent-port-0) parent also targets the bus,
    // so it is picked up BOTH by the top-level scan and by `add_sub_tracks`
    // reaching its taps. Port 0 carries nothing, so if any tap's audio
    // were summed twice this stem would be ~6 dB hotter.
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.set_output(1, TrackOutput::Bus(7));
    state.set_output(10, TrackOutput::Bus(7));
    state.set_output(11, TrackOutput::Bus(7));

    let filter = stem_filter(StemSource::Bus(7), &state.tracks.read());
    assert_eq!(
        filter.set.len(),
        3,
        "parent + two taps, each exactly once: {:?}",
        filter.set
    );

    let doubled = state.render(StemSource::Bus(7));
    assert_eq!(baseline.len(), doubled.len());
    let (a, b) = (peak(&baseline), peak(&doubled));
    assert!(a > 0.05, "baseline carries audio, got {a}");
    assert!(
        (a - b).abs() < 1e-6,
        "no double-count: peak {a} (parent on master) vs {b} (parent on the bus)"
    );
}

/// A tap routed to a different bus leaves the first bus's stem and joins
/// the other one; the instrument's own track stem keeps both.
#[test]
fn a_tap_routed_elsewhere_moves_between_bus_stems() {
    let state = EngineState::new();
    state.add_bus(7, "Kit Bus");
    state.add_bus(8, "Other Bus");
    state.set_output(10, TrackOutput::Bus(7));
    state.set_output(11, TrackOutput::Bus(8));

    let seven = state.render(StemSource::Bus(7));
    let eight = state.render(StemSource::Bus(8));
    let both = state.render(StemSource::Track(1));

    // Tap A is louder than tap B (PORT_LEVELS), so the two bus stems are
    // distinguishable and neither carries the other's audio.
    let (p7, p8, pall) = (peak(&seven), peak(&eight), peak(&both));
    assert!(p7 > 0.05 && p8 > 0.05, "each bus carries its own tap");
    assert!(
        p7 > p8 * 1.5,
        "bus 7 has the louder tap ({p7}) and bus 8 the quieter ({p8})"
    );
    assert!(
        pall > p7,
        "the instrument's own stem ({pall}) carries both taps, more than either bus ({p7})"
    );
}
