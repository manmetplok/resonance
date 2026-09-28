//! Parallel rendering is bit-identical to serial rendering
//! (realtime-multithreading.md §3, §7).
//!
//! The render pool runs each top-level track as a job on whichever thread
//! claims it, and a serial reduction replays the sums in track order. If
//! that is right, the output cannot depend on the thread count or on the
//! order jobs happened to run in — to the bit. This drives the whole audio
//! callback over a fixture meant to catch every way it could be wrong:
//!
//! - a stateful effect on every chain (a one-pole filter), so a job that
//!   rendered the wrong buffer, or a chain run twice or skipped, shows up
//!   in every later block rather than just one;
//! - five busses over three levels, two per level so bus jobs run in
//!   parallel too, fed by tracks and by other busses' aux sends, pre- and
//!   post-fader, so both reductions' orders are exercised;
//! - sidechain keys from an audible track and from a MUTED one (rendered
//!   key-only), read by filters that fold the key into their output;
//! - a multi-output instrument whose sub-tracks run their own chains, one
//!   routed to a bus;
//! - plugin-delay compensation on tracks, a sub-track and a bus;
//! - a frozen track, and a loop seam mid-buffer.
//!
//! The serial render is the reference. Each parallel run shuffles the order
//! jobs are claimed in every block (a test hook), so a pass cannot come from
//! a lucky schedule.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;

use resonance_audio::test_support::{
    __instance_from_raw_for_test, LatencyComp, MixAudioHarness, PluginSlot,
};
use resonance_audio::types::*;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

use crate::multi_out_harness::multi_out_instrument;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const BLOCKS: usize = 32;

const FEEDER: BusId = 10;
const RETURN: BusId = 20;
/// Three more busses so every bus level holds two busses, which then run
/// as parallel jobs: level 0 = FEEDER, GROUP; level 1 = RETURN (from
/// FEEDER), MID (from GROUP and FEEDER); level 2 = SUM (from MID, RETURN).
const GROUP: BusId = 60;
const MID: BusId = 70;
const SUM: BusId = 80;
const PARENT: TrackId = 50;
const TAP_A: TrackId = 51;
const TAP_B: TrackId = 52;
const INSTRUMENT: PluginInstanceId = 500;
const MUTED_KEY: TrackId = 6;
const FROZEN: TrackId = 11;
const AUDIO_TRACKS: u64 = 12;

// ---------------------------------------------------------------------------
// A stateful effect that folds its key into its output
// ---------------------------------------------------------------------------

struct FilterState {
    coef: f32,
    y: [f32; 2],
}

unsafe fn filter_state<'a>(plugin: *const clap_plugin) -> &'a mut FilterState {
    &mut *((*plugin).plugin_data as *mut FilterState)
}

unsafe extern "C" fn f_init(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn f_destroy(_: *const clap_plugin) {}
unsafe extern "C" fn f_activate(_: *const clap_plugin, _: f64, _: u32, _: u32) -> bool {
    true
}
unsafe extern "C" fn f_deactivate(_: *const clap_plugin) {}
unsafe extern "C" fn f_start(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn f_stop(_: *const clap_plugin) {}
unsafe extern "C" fn f_reset(_: *const clap_plugin) {}
unsafe extern "C" fn f_main_thread(_: *const clap_plugin) {}

/// `y += coef * (x + 0.5 * key - y)` per channel, in place. The state
/// carries across blocks; the key (input port 1, when the host connected
/// one) is mixed into what the filter tracks.
unsafe extern "C" fn f_process(plugin: *const clap_plugin, process: *const clap_process) -> i32 {
    let state = filter_state(plugin);
    let p = &*process;
    let frames = p.frames_count as usize;
    let key_chan = |ch: usize| -> Option<*const f32> {
        if p.audio_inputs_count < 2 || p.audio_inputs.is_null() {
            return None;
        }
        let key: &clap_audio_buffer = &*p.audio_inputs.add(1);
        if key.data32.is_null() || ch >= key.channel_count as usize {
            return None;
        }
        let chan = *key.data32.add(ch);
        (!chan.is_null()).then_some(chan as *const f32)
    };
    if p.audio_outputs_count < 1 || p.audio_outputs.is_null() {
        return 1;
    }
    let out: &clap_audio_buffer = &*p.audio_outputs;
    let input: &clap_audio_buffer = &*p.audio_inputs;
    for ch in 0..(out.channel_count as usize).min(2) {
        let dst = *out.data32.add(ch);
        let src = *input.data32.add(ch);
        let key = key_chan(ch);
        let mut y = state.y[ch];
        for f in 0..frames {
            let k = key.map_or(0.0, |k| *k.add(f));
            y += state.coef * (*src.add(f) + 0.5 * k - y);
            *dst.add(f) = y;
        }
        state.y[ch] = y;
    }
    1
}

unsafe extern "C" fn f_ports_count(_: *const clap_plugin, is_input: bool) -> u32 {
    if is_input {
        2
    } else {
        1
    }
}

unsafe extern "C" fn f_ports_get(
    _: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    let limit = if is_input { 2 } else { 1 };
    if index >= limit || info.is_null() {
        return false;
    }
    let out = &mut *info;
    out.id = index as clap_id;
    out.name = [0; 256];
    out.flags = 0;
    out.channel_count = 2;
    out.port_type = ptr::null();
    // In place: the host hands the same buffer as input and output.
    out.in_place_pair = if index == 0 { 0 } else { u32::MAX };
    true
}

static FILTER_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(f_ports_count),
    get: Some(f_ports_get),
};

unsafe extern "C" fn f_get_extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    if !id.is_null() && CStr::from_ptr(id) == CStr::from_ptr(CLAP_EXT_AUDIO_PORTS.as_ptr()) {
        return &FILTER_PORTS as *const clap_plugin_audio_ports as *const c_void;
    }
    ptr::null()
}

fn filter(coef: f32) -> PluginSlot {
    let inst = __instance_from_raw_for_test(
        move |_host| {
            let state = Box::into_raw(Box::new(FilterState { coef, y: [0.0; 2] }));
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(f_init),
                destroy: Some(f_destroy),
                activate: Some(f_activate),
                deactivate: Some(f_deactivate),
                start_processing: Some(f_start),
                stop_processing: Some(f_stop),
                reset: Some(f_reset),
                process: Some(f_process),
                get_extension: Some(f_get_extension),
                on_main_thread: Some(f_main_thread),
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        SR,
    )
    .expect("filter effect builds");
    assert!(inst.has_sidechain_input(), "the filter declares a key port");
    PluginSlot::new(inst)
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

fn noise(len: usize, seed: u32) -> Vec<f32> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

fn clip(id: ClipId, track_id: TrackId, seed: u32) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(noise(BLOCK * (BLOCKS + 8) * 2, seed)),
        name: format!("c{id}"),
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

fn fx_id(track: TrackId) -> PluginInstanceId {
    100 + track
}

/// A fresh fixture: new plugin instances (their filter state starts at
/// zero) over the same project every time.
fn fixture(threads: usize, shuffle_seed: u64) -> MixAudioHarness {
    let mut tracks = Vec::new();
    let mut clips = Vec::new();
    let mut plugins: Vec<(PluginInstanceId, PluginSlot)> = Vec::new();
    for id in 1..=AUDIO_TRACKS {
        let mut t = Track::new(id, format!("t{id}"));
        t.set_volume(0.3 + 0.05 * id as f32);
        t.set_pan(((id as f32) * 0.37).sin() * 0.8);
        if (3..=5).contains(&id) {
            t.set_output(TrackOutput::Bus(FEEDER));
        }
        if id == 10 || id == 12 {
            t.set_output(TrackOutput::Bus(GROUP));
        }
        if id == MUTED_KEY {
            t.set_muted(true);
        }
        if id != FROZEN {
            t.push_plugin(fx_id(id));
            plugins.push((fx_id(id), filter(0.1 + 0.03 * id as f32)));
            clips.push(clip(id, id, 7 * id as u32 + 1));
        }
        tracks.push(t);
    }
    let parent = Track::with_type(PARENT, "Kit".into(), TrackType::Instrument);
    parent.push_plugin(INSTRUMENT);
    parent.push_plugin(fx_id(PARENT));
    plugins.push((INSTRUMENT, multi_out_instrument([0.2, 0.25, 0.125])));
    plugins.push((fx_id(PARENT), filter(0.2)));
    tracks.push(parent);
    for (id, port) in [(TAP_A, 1u32), (TAP_B, 2)] {
        let mut sub = Track::new_sub_track(id, format!("Tap {port}"), PARENT, port);
        sub.set_output(if id == TAP_A {
            TrackOutput::Bus(FEEDER)
        } else {
            TrackOutput::Master
        });
        sub.push_plugin(fx_id(id));
        plugins.push((fx_id(id), filter(0.05 * port as f32)));
        tracks.push(sub);
    }
    let mut feeder = Bus::new(FEEDER, "feeder".into());
    feeder.plugin_ids.push(fx_id(FEEDER));
    plugins.push((fx_id(FEEDER), filter(0.4)));
    let mut busses = vec![feeder];
    for (id, coef) in [(RETURN, 0.15), (GROUP, 0.3), (MID, 0.22), (SUM, 0.12)] {
        let mut bus = Bus::new(id, format!("bus{id}"));
        bus.plugin_ids.push(fx_id(id));
        bus.set_volume(0.9);
        plugins.push((fx_id(id), filter(coef)));
        busses.push(bus);
    }
    let send = |id, source, dest, level_db, pre_fader| AuxSend {
        id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled: true,
    };
    let sends = vec![
        send(1, SendSource::Track(1), RETURN, -6.0, true),
        send(2, SendSource::Track(2), RETURN, -3.0, false),
        send(3, SendSource::Bus(FEEDER), RETURN, -9.0, false),
        send(4, SendSource::Track(PARENT), RETURN, -12.0, false),
        send(5, SendSource::Bus(GROUP), MID, -2.0, false),
        send(6, SendSource::Bus(FEEDER), MID, -4.0, true),
        send(7, SendSource::Bus(MID), SUM, -1.0, false),
        send(8, SendSource::Bus(RETURN), SUM, -5.0, false),
        send(9, SendSource::Track(9), SUM, -8.0, true),
    ];

    let mut h = MixAudioHarness::new(
        tracks,
        busses,
        clips,
        Vec::new(),
        sends,
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    for (id, slot) in plugins {
        h.edit_plugins(|p| p.insert(id, Arc::new(slot)));
    }
    // Keys: an audible track keys track 8's filter; the MUTED track keys
    // track 7's (rendered key-only); a sub-track keys track 9's; the
    // feeder bus keys the return's.
    let route = |plugin, source| SidechainRoute {
        plugin,
        source,
        enabled: true,
    };
    h.shared().sidechain_routes.store(Arc::new(vec![
        route(fx_id(8), SendSource::Track(2)),
        route(fx_id(7), SendSource::Track(MUTED_KEY)),
        route(fx_id(9), SendSource::Track(TAP_B)),
        route(fx_id(RETURN), SendSource::Bus(FEEDER)),
        route(fx_id(SUM), SendSource::Bus(GROUP)),
        route(fx_id(GROUP), SendSource::Bus(SUM)),
    ]));
    h.set_latency_comp(LatencyComp::new(
        48,
        &[(1, 48), (7, 16), (TAP_A, 32), (PARENT, 8)],
        24,
        &[(FEEDER, 24), (GROUP, 10), (MID, 4)],
    ));
    let frozen = noise((BLOCKS + 8) * BLOCK * 2, 99);
    let frames = frozen.len() as u64 / 2;
    let cache_ref = FreezeCacheRef::new("frozen.wav".into(), SR, 32, 1, FreezeCacheStatus::Frozen);
    h.tracks()
        .get(&FROZEN)
        .unwrap()
        .frozen_source
        .store(Some(Arc::new(FrozenSource::new(
            cache_ref,
            Arc::new(frozen),
            SR,
            frames,
        ))));

    let shared = h.shared();
    shared.playing.store(true, Ordering::Relaxed);
    shared
        .master_volume_bits
        .store(0.7f32.to_bits(), Ordering::Relaxed);
    shared.loop_enabled.store(true, Ordering::Relaxed);
    shared.loop_in.store(64, Ordering::Relaxed);
    shared
        .loop_out
        .store((BLOCK * 9 + 37) as u64, Ordering::Relaxed);

    h.set_render_threads(threads, shuffle_seed);
    h
}

/// Everything a render publishes, as raw bits: every output sample, then
/// per block the track meters and ramp state.
fn render(threads: usize, shuffle_seed: u64) -> Vec<u32> {
    render_with(&mut fixture(threads, shuffle_seed))
}

fn render_with(h: &mut MixAudioHarness) -> Vec<u32> {
    let mut bits = Vec::new();
    for _ in 0..BLOCKS {
        bits.extend(h.render().iter().map(|s| s.to_bits()));
        for (l, r) in h.take_track_peaks().into_iter().chain(h.track_last_gains()) {
            bits.push(l.to_bits());
            bits.push(r.to_bits());
        }
    }
    bits
}

#[test]
fn parallel_render_is_bit_identical_to_serial() {
    let serial = render(1, 0);
    let audible = serial
        .iter()
        .take(BLOCKS * BLOCK * 2)
        .filter(|&&b| f32::from_bits(b) != 0.0)
        .count();
    assert!(
        audible > BLOCKS * BLOCK,
        "the fixture must be audible, or it proves nothing ({audible} non-zero samples)"
    );
    // The same schedule twice must of course agree; this pins that the
    // fixture itself is deterministic before blaming the pool.
    assert_eq!(serial, render(1, 0), "serial render is not deterministic");

    for threads in [2, 3, 8] {
        for seed in 1..=4u64 {
            let parallel = render(threads, seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            if let Some(i) = serial.iter().zip(&parallel).position(|(a, b)| a != b) {
                panic!(
                    "{threads} threads, seed {seed}: first difference at word {i} \
                     (block {}): serial {} vs parallel {}",
                    i / (BLOCK * 2 + 64),
                    f32::from_bits(serial[i]),
                    f32::from_bits(parallel[i]),
                );
            }
            assert_eq!(serial.len(), parallel.len());
        }
    }
}

/// The pool really spread the jobs: with several threads the job phase's
/// summed work exceeds its wall time at least once — impossible on one
/// thread. Guards against a pool that silently always ran serially,
/// which the bit-identity test above cannot tell apart from a working one.
#[test]
fn a_parallel_pool_reports_its_threads() {
    let mut h = fixture(4, 0);
    let mut threads_seen = 0;
    for _ in 0..BLOCKS {
        h.render();
        let stats = h.take_pass_stats();
        threads_seen = threads_seen.max(stats.threads);
        assert!(stats.critical_ns <= stats.jobs_ns);
        assert!(stats.critical_track.is_some(), "some job ran");
    }
    assert_eq!(threads_seen, 4, "jobs ran on the caller plus three workers");

    let mut serial = fixture(1, 0);
    serial.render();
    assert_eq!(serial.take_pass_stats().threads, 1);
}

/// Workers that cannot take the audio thread's scheduling class — on a
/// system without an rtprio limit, say — must not be used at all: a
/// preempted worker stalls the join, which is worse than rendering
/// serially (realtime-multithreading.md §4.3). The pool reports it and
/// falls back to the audio thread alone, with the same output.
#[test]
fn workers_denied_realtime_fall_back_to_serial() {
    use resonance_audio::test_support::PoolHealth;

    let serial = render(1, 0);
    let mut h = fixture(1, 0);
    h.set_render_pool(4, 0, true);
    let bits = render_with(&mut h);
    assert!(
        bits == serial,
        "the fallback renders exactly what serial does"
    );

    let status = h.render_pool_status();
    assert_eq!(status.health, PoolHealth::RealtimeDenied { errno: 1 });
    assert_eq!(status.effective_threads, 1);
    assert_eq!(status.workers, 3);
    h.take_pass_stats();
    h.render();
    assert_eq!(h.take_pass_stats().threads, 1, "no job reaches a worker");
}

/// Tearing a pool down must never hang, whether its workers are spinning
/// right after a block or parked after a pause — the failure mode of a
/// worker lifecycle bug is a hang, not an error, so the drop runs under a
/// wall-clock watchdog (as the plugin-editor lifecycle tests do).
#[test]
fn pools_tear_down_promptly_spinning_or_parked() {
    use std::sync::mpsc;
    use std::time::Duration;

    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for round in 0..12 {
            let mut h = fixture(4, round);
            for _ in 0..4 {
                h.render();
            }
            if round % 2 == 1 {
                // Past the idle spin: every worker has parked.
                std::thread::sleep(Duration::from_millis(5));
                h.render();
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(h);
        }
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("render pools did not tear down within 30 s: a worker is wedged");
}

/// A pool whose workers parked (idle longer than their spin window) wakes
/// them again, and renders the same thing it would have spinning.
#[test]
fn parked_workers_wake_for_the_next_block() {
    use std::time::Duration;

    let serial = render(1, 0);
    let mut h = fixture(4, 7);
    let mut bits = Vec::new();
    for block in 0..BLOCKS {
        if block % 4 == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
        bits.extend(h.render().iter().map(|s| s.to_bits()));
        for (l, r) in h.take_track_peaks().into_iter().chain(h.track_last_gains()) {
            bits.push(l.to_bits());
            bits.push(r.to_bits());
        }
    }
    assert!(
        bits == serial,
        "parked-and-woken workers render what serial does"
    );
}

/// Offline renders spread over their own pool (realtime-multithreading.md
/// §4.7) and must come out bit-identical to a serial render: the master
/// stem, a bus stem and a track stem of the same fixture, rendered serial
/// and on 4 threads, each from fresh plugin state.
#[test]
fn offline_stems_are_bit_identical_across_thread_counts() {
    use resonance_audio::test_support::{
        override_threads_on_this_thread, render_stem, AutomationSnapshot, StemSource,
    };

    let render = |threads: usize, source: StemSource| -> Vec<u32> {
        override_threads_on_this_thread(Some(threads));
        let h = fixture(1, 0);
        h.shared().playing.store(false, Ordering::Relaxed);
        let tempo = Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default()));
        let out = render_stem(
            source,
            0,
            (BLOCKS * BLOCK) as u64 * 3,
            &h.shared_arc(),
            &tempo,
            &AutomationSnapshot::default(),
            SR,
        )
        .expect("stem renders");
        override_threads_on_this_thread(None);
        out.iter().map(|s| s.to_bits()).collect()
    };
    for source in [StemSource::Master, StemSource::Bus(FEEDER), StemSource::Track(PARENT)] {
        let serial = render(1, source);
        assert!(
            serial.iter().any(|&b| f32::from_bits(b) != 0.0),
            "{source:?}: the stem must be audible"
        );
        let parallel = render(4, source);
        assert!(serial == parallel, "{source:?}: parallel offline render differs");
    }
}
