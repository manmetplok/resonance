//! Plugin lock contention between the engine thread and the render (code
//! review 2026-10-02 batch R2: HOST-02 / RT-07, HOST-03, HOST-08).
//!
//! The audio thread never waits for a plugin's lock: a live block that
//! finds it held skips the plugin — an effect passes dry, an instrument is
//! silent for the block. So every moment the engine thread holds an
//! instance lock is a chance of a dropout. These pin the three ways it
//! used to hold one for nothing, or for too long:
//!
//! - the host-request poll locked every instance after every command and
//!   on every tick, with nothing pending (HOST-02);
//! - state saves serialised the plugin's state under the lock (HOST-03);
//! - and a miss left no trace anywhere (RT-07).
//!
//! Plus the thread-role half of HOST-08: `is_main_thread()` is true only
//! on host threads, and an unlocked save cannot overlap another thread's
//! main-thread call on the same instance.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::ext::render::{clap_plugin_render, clap_plugin_render_mode, CLAP_EXT_RENDER};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::ext::thread_check::{clap_host_thread_check, CLAP_EXT_THREAD_CHECK};
use clap_sys::host::clap_host;
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use clap_sys::stream::{clap_istream, clap_ostream};

use resonance_audio::test_support::{
    __instance_from_raw_for_test, plugin_lock_misses, EngineHandlerHarness, MixAudioHarness,
    PluginSlot,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;

/// What the fake plugin observed, shared with the test.
#[derive(Default)]
struct Fake {
    host: AtomicPtrHost,
    /// How long `state.save` takes.
    save_sleep_ms: u64,
    in_save: AtomicBool,
    saves: AtomicUsize,
    /// `render.set` calls that landed while a save was in flight.
    overlapping_render_sets: AtomicUsize,
    render_sets: AtomicUsize,
    /// `render.set` calls made on a thread the host reported as main.
    render_sets_on_main: AtomicUsize,
    main_thread_callbacks: AtomicUsize,
}

#[derive(Default)]
struct AtomicPtrHost(std::sync::atomic::AtomicPtr<clap_host>);

impl Fake {
    fn host(&self) -> *const clap_host {
        self.host.0.load(Ordering::Acquire)
    }
}

unsafe fn fake<'a>(p: *const clap_plugin) -> &'a Fake {
    &*((*p).plugin_data as *const Fake)
}

/// The host's `clap.thread-check`, as the plugin sees it.
unsafe fn host_thread_check(host: *const clap_host) -> &'static clap_host_thread_check {
    let get = (*host).get_extension.expect("host get_extension");
    let ext = get(host, CLAP_EXT_THREAD_CHECK.as_ptr()) as *const clap_host_thread_check;
    ext.as_ref().expect("the host serves thread-check")
}

unsafe fn host_says_main(host: *const clap_host) -> bool {
    host_thread_check(host).is_main_thread.unwrap()(host)
}

unsafe extern "C" fn c_init(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn c_destroy(_: *const clap_plugin) {}
unsafe extern "C" fn c_activate(_: *const clap_plugin, _: f64, _: u32, _: u32) -> bool {
    true
}
unsafe extern "C" fn c_deactivate(_: *const clap_plugin) {}
unsafe extern "C" fn c_start(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn c_stop(_: *const clap_plugin) {}
unsafe extern "C" fn c_reset(_: *const clap_plugin) {}
unsafe extern "C" fn c_main_thread(p: *const clap_plugin) {
    fake(p).main_thread_callbacks.fetch_add(1, Ordering::SeqCst);
}
unsafe extern "C" fn c_process(_: *const clap_plugin, _: *const clap_process) -> i32 {
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

/// A save that takes `save_sleep_ms` — a big user-wavetable state.
unsafe extern "C" fn c_state_save(p: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let f = fake(p);
    f.in_save.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(f.save_sleep_ms));
    let bytes = b"fake-state";
    let write = (*stream).write.expect("ostream write");
    let written = write(stream, bytes.as_ptr() as *const c_void, bytes.len() as u64);
    let ok = written == bytes.len() as i64;
    f.in_save.store(false, Ordering::SeqCst);
    f.saves.fetch_add(1, Ordering::SeqCst);
    ok
}
unsafe extern "C" fn c_state_load(_: *const clap_plugin, _: *const clap_istream) -> bool {
    true
}
static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(c_state_save),
    load: Some(c_state_load),
};

unsafe extern "C" fn c_render_hard_rt(_: *const clap_plugin) -> bool {
    false
}
unsafe extern "C" fn c_render_set(p: *const clap_plugin, _: clap_plugin_render_mode) -> bool {
    let f = fake(p);
    if f.in_save.load(Ordering::SeqCst) {
        f.overlapping_render_sets.fetch_add(1, Ordering::SeqCst);
    }
    if host_says_main(f.host()) {
        f.render_sets_on_main.fetch_add(1, Ordering::SeqCst);
    }
    f.render_sets.fetch_add(1, Ordering::SeqCst);
    true
}
static RENDER: clap_plugin_render = clap_plugin_render {
    has_hard_realtime_requirement: Some(c_render_hard_rt),
    set: Some(c_render_set),
};

unsafe extern "C" fn c_get_extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_AUDIO_PORTS {
        return &PORTS as *const clap_plugin_audio_ports as *const c_void;
    }
    if id == CLAP_EXT_STATE {
        return &STATE as *const clap_plugin_state as *const c_void;
    }
    if id == CLAP_EXT_RENDER {
        return &RENDER as *const clap_plugin_render as *const c_void;
    }
    ptr::null()
}

/// A fake effect whose save takes `save_sleep_ms`, plus what it observes.
fn fake_effect(save_sleep_ms: u64) -> (PluginSlot, Arc<Fake>) {
    let fake = Arc::new(Fake {
        save_sleep_ms,
        ..Fake::default()
    });
    let data = Arc::clone(&fake);
    let inst = __instance_from_raw_for_test(
        move |host| {
            data.host.0.store(host as *mut clap_host, Ordering::Release);
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                // Leaked with the plugin box: a test-lifetime fake.
                plugin_data: Arc::into_raw(data) as *mut c_void,
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
    .expect("fake effect builds");
    (PluginSlot::new(inst), fake)
}

const TRACK: TrackId = 1;
const FIRST: PluginInstanceId = 7_000;

// ---------------------------------------------------------------------------
// HOST-02: the poll locks only an instance with something pending
// ---------------------------------------------------------------------------

/// The review's verification: a burst of commands with many instances
/// live must not cost the render a single block. A second thread stands in
/// for the audio thread, `try_lock`ing every slot as fast as it can and
/// counting a miss whenever the engine side holds one.
#[test]
fn a_command_burst_never_contends_an_idle_plugin() {
    const INSTANCES: u64 = 20;
    let mut h = EngineHandlerHarness::new();
    h.push_track(Track::new(TRACK, "T1".to_string()));
    let slots: Vec<Arc<PluginSlot>> = (0..INSTANCES)
        .map(|i| h.insert_plugin_slot(FIRST + i, fake_effect(0).0))
        .collect();
    // A new instance's first poll is pending by design (its kit info is
    // read once); settle it before the "audio thread" starts.
    h.poll_plugin_host_requests();
    assert!(slots.iter().all(|s| !s.poll_pending()), "nothing pending once settled");

    let stop = Arc::new(AtomicBool::new(false));
    let blocks = Arc::new(AtomicUsize::new(0));
    let audio = {
        let (slots, stop, blocks) = (slots.clone(), Arc::clone(&stop), Arc::clone(&blocks));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                for slot in &slots {
                    drop(slot.try_lock_counted());
                }
                blocks.fetch_add(1, Ordering::Relaxed);
            }
        })
    };
    // The stand-in is running before the burst starts (a loaded machine
    // can take a while to schedule it).
    let deadline = Instant::now() + Duration::from_secs(10);
    while blocks.load(Ordering::Relaxed) == 0 {
        assert!(Instant::now() < deadline, "the audio stand-in never ran");
        std::thread::yield_now();
    }
    for i in 0..1_000 {
        h.dispatch(AudioCommand::SetTrackVolume {
            track_id: TRACK,
            volume: (i % 100) as f32 / 100.0,
        });
        // What the engine loop used to do after every single command.
        h.poll_plugin_host_requests();
    }
    stop.store(true, Ordering::Relaxed);
    audio.join().expect("audio stand-in");
    let misses: u64 = slots.iter().map(|s| s.lock_misses()).sum();
    assert_eq!(
        misses, 0,
        "the poll took an idle instance's lock {misses} time(s) during the burst"
    );
}

/// The gate must not lose a request: a callback asked for while the lock
/// is held elsewhere is delivered by the next poll that can take it.
#[test]
fn a_pending_request_survives_a_busy_lock() {
    let mut h = EngineHandlerHarness::new();
    let (slot, fake) = fake_effect(0);
    let slot = h.insert_plugin_slot(FIRST, slot);
    h.poll_plugin_host_requests();

    // The plugin asks for `on_main_thread` (`clap_host.request_callback`).
    let host = fake.host();
    unsafe { (*host).request_callback.unwrap()(host) };
    assert!(slot.poll_pending(), "the request raises the lock-free flag");

    // Another thread holds the instance through one poll.
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = {
        let slot = Arc::clone(&slot);
        std::thread::spawn(move || {
            let _guard = slot.lock();
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
    };
    held_rx.recv().unwrap();
    h.poll_plugin_host_requests();
    assert_eq!(fake.main_thread_callbacks.load(Ordering::SeqCst), 0);
    assert!(slot.poll_pending(), "a busy lock keeps the request pending");
    release_tx.send(()).unwrap();
    holder.join().unwrap();

    h.poll_plugin_host_requests();
    assert_eq!(fake.main_thread_callbacks.load(Ordering::SeqCst), 1);
    assert!(!slot.poll_pending());
    h.poll_plugin_host_requests();
    assert_eq!(
        fake.main_thread_callbacks.load(Ordering::SeqCst),
        1,
        "delivered once"
    );
}

// ---------------------------------------------------------------------------
// RT-07: a miss is counted
// ---------------------------------------------------------------------------

/// A live block that finds an effect's lock held skips it — and says so,
/// on the slot and in the process-wide count the DSP-load line prints.
#[test]
fn a_held_effect_lock_is_counted_as_a_miss() {
    const FX: PluginInstanceId = 7_100;
    let track = Track::new(TRACK, "T1".to_string());
    track.push_plugin(FX);
    let mut h = MixAudioHarness::new(
        vec![track],
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
    let slot = Arc::new(fake_effect(0).0);
    h.edit_plugins(|p| p.insert(FX, Arc::clone(&slot)));
    h.shared().playing.store(true, Ordering::Relaxed);

    h.render();
    assert_eq!(slot.lock_misses(), 0, "an uncontended block takes the lock");

    let before = plugin_lock_misses();
    {
        let _held = slot.lock();
        h.render();
    }
    assert_eq!(slot.lock_misses(), 1, "the held block is one miss");
    assert!(plugin_lock_misses() > before, "and reaches the DSP-load count");

    h.render();
    assert_eq!(slot.lock_misses(), 1, "released: no further misses");
}

// ---------------------------------------------------------------------------
// HOST-03: saves serialise outside the lock
// ---------------------------------------------------------------------------

/// While the plugin is inside a slow `state.save`, the render must still
/// be able to take its lock — for a project save (all states), a single
/// state save and a preset save alike.
#[test]
fn state_saves_do_not_hold_the_instance_lock() {
    let saves = [
        AudioCommand::SaveAllPluginStates,
        AudioCommand::SavePluginState { instance_id: FIRST },
        AudioCommand::SavePluginPresetState { instance_id: FIRST },
    ];
    for cmd in saves {
        let label = format!("{cmd:?}");
        let mut h = EngineHandlerHarness::new();
        let (slot, fake) = fake_effect(50);
        let slot = h.insert_plugin_slot(FIRST, slot);
        let probe = {
            let (slot, fake) = (Arc::clone(&slot), Arc::clone(&fake));
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !fake.in_save.load(Ordering::SeqCst) {
                    assert!(Instant::now() < deadline, "the save never started");
                    std::thread::yield_now();
                }
                slot.try_lock().is_some()
            })
        };
        h.dispatch(cmd);
        assert!(
            probe.join().expect("probe"),
            "{label}: the instance lock was held through the plugin's save"
        );
        assert_eq!(fake.saves.load(Ordering::SeqCst), 1, "{label}: saved once");
        let events = h.drain_events();
        let delivered = events.iter().any(|e| match e {
            AudioEvent::AllPluginStatesSaved { states } => {
                states.iter().any(|(id, data)| *id == FIRST && data == b"fake-state")
            }
            AudioEvent::PluginStateSaved { instance_id, data } => {
                *instance_id == FIRST && data == b"fake-state"
            }
            AudioEvent::PluginPresetStateSaved {
                instance_id, data, ..
            } => *instance_id == FIRST && data == b"fake-state",
            _ => false,
        });
        assert!(delivered, "{label}: the state reaches the app: {events:?}");
    }
}

// ---------------------------------------------------------------------------
// HOST-08: main-thread role
// ---------------------------------------------------------------------------

/// `is_main_thread()` is true on the thread that created the instance and
/// false on a thread the host never made — a plugin's own loader or GUI
/// thread. It used to be true on anything that was not an audio thread.
#[test]
fn only_host_threads_are_told_they_are_main() {
    let (_slot, fake) = fake_effect(0);
    let host = fake.host();
    assert!(unsafe { host_says_main(host) }, "the creating thread is main");
    let host_addr = host as usize;
    let on_plugin_thread = std::thread::spawn(move || unsafe {
        host_says_main(host_addr as *const clap_host)
    })
    .join()
    .unwrap();
    assert!(!on_plugin_thread, "a plugin-owned thread is not the main thread");
}

/// An unlocked save on the engine thread and an offline render's
/// `render.set` on a bounce thread are both main-thread calls on the same
/// instance; they must not overlap. And the bounce thread is told it is
/// main while it makes the call.
#[test]
fn an_unlocked_save_excludes_a_bounce_threads_render_set() {
    let mut h = EngineHandlerHarness::new();
    let (slot, fake) = fake_effect(50);
    let slot = h.insert_plugin_slot(FIRST, slot);
    let bounce = {
        let (slot, fake) = (Arc::clone(&slot), Arc::clone(&fake));
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !fake.in_save.load(Ordering::SeqCst) {
                assert!(Instant::now() < deadline, "the save never started");
                std::thread::yield_now();
            }
            // What `OfflineRenderMode::enter` does on the bounce thread.
            let mut inst = slot.lock();
            inst.0.set_render_mode(true)
        })
    };
    h.dispatch(AudioCommand::SavePluginState { instance_id: FIRST });
    assert!(bounce.join().expect("bounce stand-in"), "render.set accepted");
    assert_eq!(fake.render_sets.load(Ordering::SeqCst), 1);
    assert_eq!(
        fake.overlapping_render_sets.load(Ordering::SeqCst),
        0,
        "render.set ran inside the unlocked state save"
    );
    assert_eq!(
        fake.render_sets_on_main.load(Ordering::SeqCst),
        1,
        "the offline renderer's thread is main for render.set"
    );
}
