//! CLAP thread roles, as a plugin that checks them sees them
//! (realtime-multithreading.md §4.6).
//!
//! The host serves `clap.thread-check`, so a plugin can ask on every call
//! whether it is on an audio or the main thread — and some abort when the
//! answer is wrong for the call (u-he Hive: "Host called the method
//! clap_plugin.start_processing() on wrong thread!"). CLAP makes
//! `start_processing`, `stop_processing`, `reset` and `process`
//! audio-thread calls and the lifecycle calls main-thread ones, even
//! though the host makes some audio-thread calls from its engine thread
//! and `process()` now moves between render workers. This fake plugin
//! checks the role on every one of those calls, through a whole life:
//! create, render on one thread and on a pool, reset, restart, drop.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::ext::thread_check::{clap_host_thread_check, CLAP_EXT_THREAD_CHECK};
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;

use resonance_audio::test_support::{__instance_from_raw_for_test, MixAudioHarness, PluginSlot};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const FX: PluginInstanceId = 800;

struct Checker {
    host: *const clap_host,
    violations: Arc<Mutex<Vec<String>>>,
    process_calls: Arc<Mutex<usize>>,
}

unsafe fn checker<'a>(plugin: *const clap_plugin) -> &'a Checker {
    &*((*plugin).plugin_data as *const Checker)
}

/// Record a violation unless the host reports the role `call` requires.
unsafe fn expect(plugin: *const clap_plugin, call: &str, audio: bool) {
    let c = checker(plugin);
    let Some(get) = (*c.host).get_extension else {
        return;
    };
    let ext = get(c.host, CLAP_EXT_THREAD_CHECK.as_ptr()) as *const clap_host_thread_check;
    let Some(ext) = ext.as_ref() else {
        c.violations.lock().unwrap().push("no thread-check".into());
        return;
    };
    let is_audio = ext.is_audio_thread.unwrap()(c.host);
    let is_main = ext.is_main_thread.unwrap()(c.host);
    if is_audio != audio || is_main == audio {
        c.violations.lock().unwrap().push(format!(
            "{call}: audio={is_audio} main={is_main} on {:?}",
            std::thread::current().name()
        ));
    }
}

unsafe extern "C" fn c_init(p: *const clap_plugin) -> bool {
    expect(p, "init", false);
    true
}
unsafe extern "C" fn c_destroy(p: *const clap_plugin) {
    expect(p, "destroy", false);
}
unsafe extern "C" fn c_activate(p: *const clap_plugin, _: f64, _: u32, _: u32) -> bool {
    expect(p, "activate", false);
    true
}
unsafe extern "C" fn c_deactivate(p: *const clap_plugin) {
    expect(p, "deactivate", false);
}
unsafe extern "C" fn c_start(p: *const clap_plugin) -> bool {
    expect(p, "start_processing", true);
    true
}
unsafe extern "C" fn c_stop(p: *const clap_plugin) {
    expect(p, "stop_processing", true);
}
unsafe extern "C" fn c_reset(p: *const clap_plugin) {
    expect(p, "reset", true);
}
unsafe extern "C" fn c_main_thread(p: *const clap_plugin) {
    expect(p, "on_main_thread", false);
}
unsafe extern "C" fn c_process(p: *const clap_plugin, _: *const clap_process) -> i32 {
    expect(p, "process", true);
    *checker(p).process_calls.lock().unwrap() += 1;
    1
}

unsafe extern "C" fn c_ports_count(_: *const clap_plugin, _: bool) -> u32 {
    1
}
unsafe extern "C" fn c_ports_get(
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
    count: Some(c_ports_count),
    get: Some(c_ports_get),
};
unsafe extern "C" fn c_get_extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    if !id.is_null() && CStr::from_ptr(id) == CLAP_EXT_AUDIO_PORTS {
        return &PORTS as *const clap_plugin_audio_ports as *const c_void;
    }
    ptr::null()
}

fn checking_effect(violations: &Arc<Mutex<Vec<String>>>, calls: &Arc<Mutex<usize>>) -> PluginSlot {
    let (violations, process_calls) = (Arc::clone(violations), Arc::clone(calls));
    let inst = __instance_from_raw_for_test(
        move |host| {
            let state = Box::into_raw(Box::new(Checker {
                host,
                violations,
                process_calls,
            }));
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(c_init),
                destroy: Some(c_destroy),
                activate: Some(c_activate),
                deactivate: Some(c_deactivate),
                start_processing: Some(c_start),
                stop_processing: Some(c_stop),
                reset: Some(c_reset),
                process: Some(c_process),
                get_extension: Some(c_get_extension),
                on_main_thread: Some(c_main_thread),
            })) as *const clap_plugin
        },
        SR,
    )
    .expect("checking effect builds");
    PluginSlot::new(inst)
}

/// Create, render (one thread, then a four-thread pool), reset, restart
/// and drop: every call arrives with the role CLAP gives it.
#[test]
fn every_plugin_call_sees_its_clap_thread_role() {
    for threads in [1, 4] {
        let violations = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(Mutex::new(0));
        let tracks: Vec<Track> = (1..=6)
            .map(|id| {
                let t = Track::new(id, format!("t{id}"));
                t.push_plugin(FX + id);
                t
            })
            .collect();
        let mut h = MixAudioHarness::new(
            tracks,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            TempoMap::default(),
            128,
            2,
            SR,
            true,
        );
        for id in 1..=6 {
            let slot = checking_effect(&violations, &calls);
            h.edit_plugins(|p| p.insert(FX + id, Arc::new(slot)));
        }
        h.shared().playing.store(true, Ordering::Relaxed);
        h.set_render_threads(threads, 0);
        for _ in 0..16 {
            h.render();
        }
        // The engine thread's own audio-thread calls: a reset, a
        // stop/start cycle, and a full deactivate/activate restart.
        for id in 1..=6 {
            let plugins = h.plugins();
            let mut inst = plugins.get(&(FX + id)).unwrap().lock();
            inst.0.reset();
            assert!(inst.0.reset_processing());
            assert!(inst.0.restart());
        }
        h.render();
        drop(h);

        assert!(
            *calls.lock().unwrap() >= 6 * 16,
            "{threads} threads: every track processed"
        );
        let violations = violations.lock().unwrap();
        assert!(
            violations.is_empty(),
            "{threads} threads: calls on the wrong CLAP thread role:\n{}",
            violations.join("\n")
        );
    }
}
