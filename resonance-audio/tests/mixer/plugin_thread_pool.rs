//! CLAP `thread-pool`: a plugin splits its `process()` into tasks and asks
//! the host to run them (realtime-multithreading.md §5, P4 — u-he Hive
//! and MFM2 do this in their multicore modes).
//!
//! The host runs them on the render pool — the calling thread plus idle
//! workers — or inline on the calling thread when there is no pool. Either
//! way every task must have run when `request_exec` returns, so the
//! plugin's output is the same, and the request must be refused outside
//! `process()`.

use std::collections::HashSet;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::ext::thread_pool::{
    clap_host_thread_pool, clap_plugin_thread_pool, CLAP_EXT_THREAD_POOL,
};
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;

use resonance_audio::test_support::{__instance_from_raw_for_test, MixAudioHarness, PluginSlot};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const TASKS: u32 = 16;
const FX: PluginInstanceId = 700;

/// Everything a task needs, published by `process()` before it asks.
struct Split {
    input: [*const f32; 2],
    output: [*mut f32; 2],
    frames: usize,
}

struct SplitState {
    host: *const clap_host,
    split: Mutex<Option<Split>>,
    /// Threads that ran a task, for the test to count.
    threads: Arc<Mutex<HashSet<ThreadId>>>,
    /// Whether every `request_exec` inside `process()` was granted.
    granted: Arc<AtomicBool>,
    /// Tasks run in total.
    ran: Arc<AtomicU32>,
    /// A request made from `activate`, outside `process()`: CLAP says the
    /// host must refuse it.
    refused_outside_process: Arc<AtomicBool>,
}

unsafe impl Send for Split {}

unsafe fn state<'a>(plugin: *const clap_plugin) -> &'a SplitState {
    &*((*plugin).plugin_data as *const SplitState)
}

unsafe fn host_pool(host: *const clap_host) -> Option<&'static clap_host_thread_pool> {
    let get = (*host).get_extension?;
    (get(host, CLAP_EXT_THREAD_POOL.as_ptr()) as *const clap_host_thread_pool).as_ref()
}

unsafe extern "C" fn p_init(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn p_destroy(_: *const clap_plugin) {}
unsafe extern "C" fn p_activate(plugin: *const clap_plugin, _: f64, _: u32, _: u32) -> bool {
    let s = state(plugin);
    if let Some(pool) = host_pool(s.host) {
        let granted = pool.request_exec.map_or(true, |req| req(s.host, TASKS));
        s.refused_outside_process.store(!granted, Ordering::Relaxed);
    }
    true
}
unsafe extern "C" fn p_deactivate(_: *const clap_plugin) {}
unsafe extern "C" fn p_start(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn p_stop(_: *const clap_plugin) {}
unsafe extern "C" fn p_reset(_: *const clap_plugin) {}
unsafe extern "C" fn p_main_thread(_: *const clap_plugin) {}

/// Task `i` of `TASKS`: a slice of the block, `out = 0.5 * in`, after a
/// short busy wait so helpers have time to join in.
unsafe extern "C" fn p_exec(plugin: *const clap_plugin, task: u32) {
    let s = state(plugin);
    let start = Instant::now();
    while start.elapsed() < Duration::from_micros(20) {
        std::hint::spin_loop();
    }
    let split = s.split.lock().unwrap();
    let split = split.as_ref().expect("tasks run only inside process()");
    let per = split.frames.div_ceil(TASKS as usize);
    let from = (task as usize * per).min(split.frames);
    let to = (from + per).min(split.frames);
    for ch in 0..2 {
        for f in from..to {
            *split.output[ch].add(f) = 0.5 * *split.input[ch].add(f);
        }
    }
    s.threads
        .lock()
        .unwrap()
        .insert(std::thread::current().id());
    s.ran.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn p_process(plugin: *const clap_plugin, process: *const clap_process) -> i32 {
    let s = state(plugin);
    let p = &*process;
    let input: &clap_audio_buffer = &*p.audio_inputs;
    let output: &clap_audio_buffer = &*p.audio_outputs;
    *s.split.lock().unwrap() = Some(Split {
        input: [*input.data32, *input.data32.add(1)],
        output: [*output.data32, *output.data32.add(1)],
        frames: p.frames_count as usize,
    });
    let granted = host_pool(s.host)
        .and_then(|pool| pool.request_exec)
        .is_some_and(|req| req(s.host, TASKS));
    if !granted {
        s.granted.store(false, Ordering::Relaxed);
        for task in 0..TASKS {
            p_exec(plugin, task);
        }
    }
    *s.split.lock().unwrap() = None;
    1
}

unsafe extern "C" fn p_ports_count(_: *const clap_plugin, _: bool) -> u32 {
    1
}

unsafe extern "C" fn p_ports_get(
    _: *const clap_plugin,
    index: u32,
    _: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index != 0 || info.is_null() {
        return false;
    }
    let out = &mut *info;
    out.id = 0 as clap_id;
    out.name = [0; 256];
    out.flags = 0;
    out.channel_count = 2;
    out.port_type = ptr::null();
    out.in_place_pair = 0;
    true
}

static PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(p_ports_count),
    get: Some(p_ports_get),
};

static THREAD_POOL: clap_plugin_thread_pool = clap_plugin_thread_pool { exec: Some(p_exec) };

unsafe extern "C" fn p_get_extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_AUDIO_PORTS {
        return &PORTS as *const clap_plugin_audio_ports as *const c_void;
    }
    if id == CLAP_EXT_THREAD_POOL {
        return &THREAD_POOL as *const clap_plugin_thread_pool as *const c_void;
    }
    ptr::null()
}

struct Probe {
    threads: Arc<Mutex<HashSet<ThreadId>>>,
    granted: Arc<AtomicBool>,
    ran: Arc<AtomicU32>,
    refused_outside_process: Arc<AtomicBool>,
}

fn splitting_effect() -> (PluginSlot, Probe) {
    let probe = Probe {
        threads: Arc::default(),
        granted: Arc::new(AtomicBool::new(true)),
        ran: Arc::default(),
        refused_outside_process: Arc::default(),
    };
    let (threads, granted, ran, refused) = (
        Arc::clone(&probe.threads),
        Arc::clone(&probe.granted),
        Arc::clone(&probe.ran),
        Arc::clone(&probe.refused_outside_process),
    );
    let inst = __instance_from_raw_for_test(
        move |host| {
            let state = Box::into_raw(Box::new(SplitState {
                host,
                split: Mutex::new(None),
                threads,
                granted,
                ran,
                refused_outside_process: refused,
            }));
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(p_init),
                destroy: Some(p_destroy),
                activate: Some(p_activate),
                deactivate: Some(p_deactivate),
                start_processing: Some(p_start),
                stop_processing: Some(p_stop),
                reset: Some(p_reset),
                process: Some(p_process),
                get_extension: Some(p_get_extension),
                on_main_thread: Some(p_main_thread),
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        SR,
    )
    .expect("splitting effect builds");
    (PluginSlot::new(inst), probe)
}

fn render(threads: usize, blocks: usize) -> (Vec<u32>, Probe) {
    let track = Track::new(1, "t".into());
    track.push_plugin(FX);
    let samples: Vec<f32> = (0..BLOCK * (blocks + 4) * 2)
        .map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5)
        .collect();
    let clip = AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::memory(samples),
        name: "c".into(),
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
    };
    let mut h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![clip],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    let (slot, probe) = splitting_effect();
    h.edit_plugins(|p| p.insert(FX, Arc::new(slot)));
    h.shared().playing.store(true, Ordering::Relaxed);
    h.set_render_threads(threads, 0);
    let mut bits = Vec::new();
    for _ in 0..blocks {
        bits.extend(h.render().iter().map(|s| s.to_bits()));
    }
    (bits, probe)
}

#[test]
fn plugin_tasks_run_on_the_pool_with_the_same_output() {
    let blocks = 24;
    let (serial, serial_probe) = render(1, blocks);
    assert!(serial.iter().any(|&b| f32::from_bits(b) != 0.0), "audible");
    assert!(
        serial_probe.granted.load(Ordering::Relaxed),
        "granted, run inline"
    );
    assert_eq!(
        serial_probe.ran.load(Ordering::Relaxed),
        TASKS * blocks as u32
    );
    assert_eq!(
        serial_probe.threads.lock().unwrap().len(),
        1,
        "no pool: one thread"
    );

    let (parallel, probe) = render(4, blocks);
    assert!(parallel == serial, "the pool changed the plugin's output");
    assert!(probe.granted.load(Ordering::Relaxed));
    assert_eq!(probe.ran.load(Ordering::Relaxed), TASKS * blocks as u32);
    assert!(
        probe.threads.lock().unwrap().len() > 1,
        "idle workers help with a plugin's tasks"
    );
}

#[test]
fn requests_outside_process_are_refused() {
    let (_, probe) = render(1, 1);
    assert!(
        probe.refused_outside_process.load(Ordering::Relaxed),
        "a request from activate() must be refused"
    );
}
