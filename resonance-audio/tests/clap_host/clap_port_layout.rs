//! Every `process()` hands the plugin exactly the audio ports it
//! declared, each with its declared channel count (code review HOST-05).
//!
//! `clap/process.h` ties `audio_inputs_count` / `audio_outputs_count` to
//! `clap_plugin_audio_ports.count()` and each buffer's `channel_count` to
//! its port's, and a plugin may index every port it declared. The host
//! used to pass one input unless a key was routed, at most as many
//! outputs as its caller had buffers (8 on a track), and always two
//! channels — a third-party sidechain compressor with no key routed, or a
//! 16-out sampler, read or wrote through null.
//!
//! The fake plugin here declares a main input, a key input and a
//! three-channel aux input, and ten outputs with a mono and a 6-channel
//! port among them. Its `process` checks the layout it is handed and
//! touches every frame of every channel — recording, never dereferencing,
//! a missing pointer — through each entry point: `process`,
//! `process_multi_with_key` with and without a key, and the transport
//! panic (`Stop`), which drives the single-output wrapper on the engine
//! thread.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::Arc;

use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS};
use clap_sys::id::CLAP_INVALID_ID;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    ClapInstance, EngineHandlerHarness, PluginSlot, StereoBufMut, __instance_from_raw_for_test,
};
use resonance_audio::types::{Track, TrackType};

const INPUTS: &[u32] = &[2, 2, 3];
const OUTPUTS: &[u32] = &[2, 2, 1, 6, 2, 2, 2, 2, 2, 2];

struct PortFake {
    calls: u32,
    /// Every way a call's buffers differed from the declaration.
    violations: Vec<String>,
    /// What each call read from input port 1 (the key), summed.
    key_sum: f32,
}

unsafe fn fake<'a>(p: *const clap_plugin) -> &'a mut PortFake {
    unsafe { &mut *((*p).plugin_data as *mut PortFake) }
}

unsafe extern "C" fn ok(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn noop(_p: *const clap_plugin) {}
unsafe extern "C" fn activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}

unsafe extern "C" fn ports_count(_p: *const clap_plugin, is_input: bool) -> u32 {
    if is_input {
        INPUTS.len() as u32
    } else {
        OUTPUTS.len() as u32
    }
}

unsafe extern "C" fn ports_get(
    _p: *const clap_plugin,
    index: u32,
    is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    let layout = if is_input { INPUTS } else { OUTPUTS };
    let Some(&channels) = layout.get(index as usize) else {
        return false;
    };
    let info = unsafe { &mut *info };
    info.id = index;
    info.flags = 0;
    info.channel_count = channels;
    info.port_type = ptr::null();
    info.in_place_pair = CLAP_INVALID_ID;
    for (dst, src) in info.name.iter_mut().zip(b"port\0") {
        *dst = *src as c_char;
    }
    true
}

static PORTS_EXT: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(ports_count),
    get: Some(ports_get),
};

unsafe extern "C" fn get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_EXT_AUDIO_PORTS {
        &PORTS_EXT as *const clap_plugin_audio_ports as *const c_void
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    let s = unsafe { fake(p) };
    s.calls += 1;
    let pr = unsafe { &*process };
    let frames = pr.frames_count as usize;
    let mut check = |dir: &str, count: u32, bufs: *const clap_sys::audio_buffer::clap_audio_buffer, layout: &[u32], write: bool| {
        if count as usize != layout.len() {
            s.violations.push(format!("{dir}: {count} ports passed, {} declared", layout.len()));
        }
        if bufs.is_null() {
            if !layout.is_empty() {
                s.violations.push(format!("{dir}: null port array"));
            }
            return;
        }
        for (port, &declared) in layout.iter().enumerate().take(count as usize) {
            let buf = unsafe { &*bufs.add(port) };
            if buf.channel_count != declared {
                s.violations.push(format!(
                    "{dir} port {port}: {} channels passed, {declared} declared",
                    buf.channel_count
                ));
            }
            if buf.data32.is_null() {
                s.violations.push(format!("{dir} port {port}: null data32"));
                continue;
            }
            for ch in 0..declared.min(buf.channel_count) as usize {
                let data = unsafe { *buf.data32.add(ch) };
                if data.is_null() {
                    s.violations.push(format!("{dir} port {port} channel {ch}: null"));
                    continue;
                }
                let samples = unsafe { std::slice::from_raw_parts_mut(data, frames) };
                if write {
                    samples.fill(0.5);
                } else {
                    let sum: f32 = samples.iter().sum();
                    if port == 1 {
                        s.key_sum += sum;
                    }
                }
            }
        }
    };
    check("input", pr.audio_inputs_count, pr.audio_inputs, INPUTS, false);
    check("output", pr.audio_outputs_count, pr.audio_outputs, OUTPUTS, true);
    CLAP_PROCESS_CONTINUE
}

fn port_fake() -> (ClapInstance, *mut PortFake) {
    let mut state: *mut PortFake = ptr::null_mut();
    let inst = __instance_from_raw_for_test(
        |_host| {
            state = Box::into_raw(Box::new(PortFake {
                calls: 0,
                violations: Vec::new(),
                key_sum: 0.0,
            }));
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(ok),
                destroy: Some(noop),
                activate: Some(activate),
                deactivate: Some(noop),
                start_processing: Some(ok),
                stop_processing: Some(noop),
                reset: Some(noop),
                process: Some(process),
                get_extension: Some(get_extension),
                on_main_thread: None,
            })) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake instance");
    (inst, state)
}

fn assert_clean(state: *mut PortFake, calls: u32) {
    let s = unsafe { &*state };
    assert_eq!(s.calls, calls, "process() calls");
    assert!(s.violations.is_empty(), "port layout violations: {:#?}", s.violations);
}

#[test]
fn process_passes_every_declared_port_and_channel() {
    let (mut inst, state) = port_fake();
    let mut l = vec![0.0f32; 256];
    let mut r = vec![0.0f32; 256];
    inst.process(&mut l, &mut r, 256);
    assert_clean(state, 1);
    assert!(l.iter().chain(&r).all(|&s| s == 0.5), "main output written");
}

#[test]
fn process_multi_without_a_key_still_passes_the_key_port_as_silence() {
    let (mut inst, state) = port_fake();
    assert!(inst.has_sidechain_input());
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..3).map(|_| (vec![1.0; 128], vec![1.0; 128])).collect();
    let mut outs: Vec<StereoBufMut<'_>> = bufs
        .iter_mut()
        .map(|(l, r)| StereoBufMut { left: l, right: r })
        .collect();
    inst.process_multi_with_key(&mut outs, None, 128);
    assert_clean(state, 1);
    assert_eq!(unsafe { (*state).key_sum }, 0.0, "an unrouted key reads silence");
    // Port 2 is mono: the host mirrors its one channel into the pair.
    assert!(bufs[2].1.iter().all(|&s| s == 0.5), "mono port mirrored into right");

    // With a key routed, port 1 reads it.
    let key = vec![0.25f32; 128];
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..1).map(|_| (vec![0.0; 128], vec![0.0; 128])).collect();
    let mut outs: Vec<StereoBufMut<'_>> = bufs
        .iter_mut()
        .map(|(l, r)| StereoBufMut { left: l, right: r })
        .collect();
    inst.process_multi_with_key(&mut outs, Some((&key, &key)), 128);
    assert_clean(state, 2);
    assert_eq!(unsafe { (*state).key_sum }, 0.25 * 128.0 * 2.0);
}

/// `Stop` while playing panics every instrument through the single-output
/// `process()` on the engine thread — on a multi-out instrument too.
#[test]
fn the_transport_panic_passes_every_declared_port_and_channel() {
    let (inst, state) = port_fake();
    let mut harness = EngineHandlerHarness::new();
    harness.push_track(Track::with_type(1, "synth".into(), TrackType::Instrument));
    harness
        .shared()
        .edit_plugins(|p| p.insert(900, Arc::new(PluginSlot::new(inst))));
    let _ = harness.shared().tracks()[&1].push_plugin(900);
    harness.play();
    harness.stop();
    let calls = unsafe { (*state).calls };
    assert!(calls >= 1, "the panic must have processed the instrument");
    assert_clean(state, calls);
}

/// The key port is now always passed (above), so "no key routed" can no
/// longer mean "port left out". The host flags an unrouted key port's
/// channels constant silence, and the first-party bridge reads that as no
/// key: the real Resonance Gate, unrouted, still keys off its own input —
/// exactly as before HOST-05 — while a routed key that is silent shuts it.
#[test]
fn an_unrouted_key_still_self_keys_a_first_party_gate() {
    let Some(path) = crate::plugin_binaries::plugin_binary("resonance-gate") else {
        return;
    };
    let bundle = resonance_audio::test_support::ClapBundle::load(&path).expect("bundle load");
    let id = bundle.descriptors()[0].id.clone();

    // 0.3 s of a loud sine, in 256-frame blocks; the level of the last
    // block says whether the gate is open.
    let run = |key: Option<&[f32]>| -> f32 {
        let mut inst = bundle.create_instance(&id, 48_000).expect("instance");
        assert!(inst.has_sidechain_input());
        let mut peak = 0.0f32;
        for block in 0..56 {
            let mut l: Vec<f32> = (0..256)
                .map(|i| ((block * 256 + i) as f32 * 0.05).sin() * 0.5)
                .collect();
            let mut r = l.clone();
            let mut outs = [StereoBufMut {
                left: &mut l,
                right: &mut r,
            }];
            inst.process_multi_with_key(&mut outs, key.map(|k| (k, k)), 256);
            peak = l.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        }
        peak
    };
    let unrouted = run(None);
    let silent = vec![0.0f32; 256];
    let silent_key = run(Some(&silent));
    assert!(unrouted > 0.4, "unrouted, the gate must open on its own input: peak {unrouted}");
    assert!(
        silent_key < 0.01,
        "keyed from silence, the gate must stay shut: peak {silent_key}"
    );
}
