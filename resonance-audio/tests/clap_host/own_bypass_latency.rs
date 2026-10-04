//! A plugin's own (latency-preserving) bypass, driven by the host
//! (code review HOST-04, HOST-14, HOST-07).
//!
//! * **HOST-04** — the host used to crossfade over a plugin's own bypass
//!   too, against its *undelayed* dry copy. A plugin with latency outputs
//!   its input delayed by that latency, bypassed or not, so the host
//!   faded between two misaligned signals for 5 ms and then snapped back
//!   to the plugin's output: a step at both ends. The host now leaves the
//!   transition to the plugin (`PluginSlot::stage` is always `Wet` for an
//!   own-bypass slot).
//! * **HOST-14** — `set_param` dropped a change silently when 128 other
//!   parameters were already queued, and `sync_own_bypass` marked the
//!   bypass sent before trying: the bypass was then never delivered. Now
//!   `set_param` reports whether it queued and the bypass is retried.
//! * **HOST-07** — Mastering declares `IS_BYPASS`, so the host bypasses
//!   it through that parameter and its lookahead latency stays in the
//!   comp table.
//!
//! The fake: a 256-frame delay whose engaged output is the delayed input
//! inverted, and whose bypassed output is the delayed input — switching
//! itself over a 10 ms gain ramp, as a well-behaved plugin does.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use clap_sys::events::{
    clap_event_param_value, clap_input_events, clap_output_events, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS,
    CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_BYPASS, CLAP_PARAM_IS_STEPPED,
};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    __instance_from_raw_for_test, compensation_delays, chain_latencies, ClapBundle,
    MixAudioHarness, PluginSlot,
};
use resonance_audio::types::*;

use crate::plugin_binaries::plugin_binary;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const LATENCY: usize = 256;
const BYPASS_ID: u32 = 77;
const FX: PluginInstanceId = 900;
/// The fake's own switch: a gain ramp from −1 (engaged) to +1 (bypassed)
/// over 10 ms.
const RAMP_FRAMES: f32 = 480.0;

struct Fake {
    ring_l: [f32; LATENCY],
    ring_r: [f32; LATENCY],
    pos: usize,
    gain: f32,
    target: f32,
    bypassed: bool,
}

unsafe fn fake<'a>(p: *const clap_plugin) -> &'a mut Fake {
    unsafe { &mut *((*p).plugin_data as *mut Fake) }
}

unsafe extern "C" fn ok(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn noop(_p: *const clap_plugin) {}
unsafe extern "C" fn activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}

unsafe extern "C" fn process(p: *const clap_plugin, pr: *const clap_process) -> clap_process_status {
    unsafe {
        let st = fake(p);
        let pr = &*pr;
        let frames = pr.frames_count as usize;
        let out = &*pr.audio_outputs;
        let (l, r) = (*out.data32, *out.data32.add(1));
        let events = &*pr.in_events;
        let count = (events.size.unwrap())(events);
        let mut next = 0;
        let step = 2.0 / RAMP_FRAMES;
        for i in 0..frames {
            while next < count {
                let h = (events.get.unwrap())(events, next);
                if (*h).time as usize > i {
                    break;
                }
                if (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                    let e = &*(h as *const clap_event_param_value);
                    if e.param_id == BYPASS_ID {
                        st.bypassed = e.value >= 0.5;
                        st.target = if st.bypassed { 1.0 } else { -1.0 };
                    }
                }
                next += 1;
            }
            st.gain = if st.gain < st.target {
                (st.gain + step).min(st.target)
            } else {
                (st.gain - step).max(st.target)
            };
            let (xl, xr) = (*l.add(i), *r.add(i));
            let (yl, yr) = (st.ring_l[st.pos], st.ring_r[st.pos]);
            st.ring_l[st.pos] = xl;
            st.ring_r[st.pos] = xr;
            st.pos = (st.pos + 1) % LATENCY;
            *l.add(i) = st.gain * yl;
            *r.add(i) = st.gain * yr;
        }
    }
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn latency_get(_p: *const clap_plugin) -> u32 {
    LATENCY as u32
}
static LATENCY_EXT: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};

unsafe extern "C" fn param_count(_p: *const clap_plugin) -> u32 {
    1
}
unsafe extern "C" fn param_info(_p: *const clap_plugin, index: u32, out: *mut clap_param_info) -> bool {
    if index != 0 {
        return false;
    }
    unsafe {
        let info = &mut *out;
        info.id = BYPASS_ID;
        info.flags = CLAP_PARAM_IS_BYPASS | CLAP_PARAM_IS_STEPPED | CLAP_PARAM_IS_AUTOMATABLE;
        info.cookie = ptr::null_mut();
        info.name[0] = b'B' as c_char;
        info.name[1] = 0;
        info.module[0] = 0;
        info.min_value = 0.0;
        info.max_value = 1.0;
        info.default_value = 0.0;
    }
    true
}
unsafe extern "C" fn param_value(p: *const clap_plugin, id: u32, out: *mut f64) -> bool {
    if id != BYPASS_ID {
        return false;
    }
    unsafe { *out = f64::from(u8::from(fake(p).bypassed)) };
    true
}
unsafe extern "C" fn param_flush(
    _p: *const clap_plugin,
    _in: *const clap_input_events,
    _out: *const clap_output_events,
) {
}
static PARAMS_EXT: clap_plugin_params = clap_plugin_params {
    count: Some(param_count),
    get_info: Some(param_info),
    get_value: Some(param_value),
    value_to_text: None,
    text_to_value: None,
    flush: Some(param_flush),
};

unsafe extern "C" fn get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) }.to_bytes();
    if id == CLAP_EXT_LATENCY.to_bytes() {
        &LATENCY_EXT as *const clap_plugin_latency as *const c_void
    } else if id == CLAP_EXT_PARAMS.to_bytes() {
        &PARAMS_EXT as *const clap_plugin_params as *const c_void
    } else {
        ptr::null()
    }
}

/// The fake as a chain slot, plus its state (leaked with the plugin).
fn own_bypass_fx() -> (PluginSlot, *mut Fake) {
    let state = Box::into_raw(Box::new(Fake {
        ring_l: [0.0; LATENCY],
        ring_r: [0.0; LATENCY],
        pos: 0,
        gain: -1.0,
        target: -1.0,
        bypassed: false,
    }));
    let inst = __instance_from_raw_for_test(
        |_host| {
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
        SR,
    )
    .expect("fake own-bypass plugin");
    assert_eq!(inst.latency_samples(), LATENCY as u32);
    let slot = PluginSlot::new(inst);
    assert_eq!(slot.bypass_param, Some(BYPASS_ID), "the fake declares IS_BYPASS");
    (slot, state)
}

fn sine(n: usize) -> f32 {
    0.5 * (n as f32 * 2.0 * std::f32::consts::PI * 220.0 / SR as f32).sin()
}

fn sine_clip() -> AudioClip {
    let data: Vec<f32> = (0..200 * BLOCK).flat_map(|i| [sine(i), sine(i)]).collect();
    AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::memory(data),
        name: "sine".into(),
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

/// One track playing [`sine_clip`] through the fake, to master.
fn harness() -> (MixAudioHarness, Arc<PluginSlot>, *mut Fake) {
    let track = Track::new(1, "t".into());
    track.push_plugin(FX);
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![sine_clip()],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    let (slot, state) = own_bypass_fx();
    let slot = Arc::new(slot);
    let published = Arc::clone(&slot);
    h.edit_plugins(|p| p.insert(FX, published));
    h.shared().master_volume_bits.store(1.0f32.to_bits(), Ordering::Relaxed);
    h.shared().playing.store(true, Ordering::Relaxed);
    (h, slot, state)
}

fn left(h: &mut MixAudioHarness) -> Vec<f32> {
    h.render().chunks(2).map(|f| f[0]).collect()
}

fn max_step(xs: &[f32]) -> f32 {
    xs.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
}

#[test]
fn toggling_a_latent_plugins_own_bypass_has_no_discontinuity() {
    let (mut h, slot, state) = harness();
    let mut out = Vec::new();
    for block in 0..60 {
        match block {
            // While playing, a bypass request moves the target and lets
            // the render fade (`apply_bypass_request`).
            20 => slot.bypass.set_bypassed(true),
            40 => slot.bypass.set_bypassed(false),
            _ => {}
        }
        out.extend(left(&mut h));
        if block == 39 {
            assert!(unsafe { (*state).bypassed }, "the plugin was told to bypass itself");
        }
    }
    assert!(!unsafe { (*state).bypassed }, "and to re-engage");

    // Past the plugin's own fill and the transport's start.
    let steady = &out[4 * LATENCY..20 * BLOCK];
    let peak = steady.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.1, "not vacuous: {peak}");
    let signal_step = max_step(steady);
    // The plugin's own switch adds at most its gain ramp's slope.
    let budget = signal_step + peak * 2.0 / RAMP_FRAMES;
    let got = max_step(&out[20 * BLOCK - 1..]);
    assert!(
        got <= budget * 1.05,
        "own-bypass toggle stepped by {got} (budget {budget}): the host must not crossfade \
         a latent plugin against its undelayed input"
    );
}

#[test]
fn set_param_reports_a_full_queue() {
    let (slot, _state) = own_bypass_fx();
    let mut guard = slot.lock();
    let inst = &mut guard.0;
    let mut queued = 0u32;
    while inst.set_param(1_000 + queued, 0.5) {
        queued += 1;
        assert!(queued <= 10_000, "set_param never reports a full queue");
    }
    assert_eq!(queued as usize, 128, "the cap");
    assert!(inst.set_param(1_000, 0.25), "a change to an already-queued param always lands");
}

#[test]
fn a_bypass_that_finds_the_queue_full_is_retried() {
    let (mut h, slot, state) = harness();
    for _ in 0..4 {
        left(&mut h);
    }
    {
        let mut guard = slot.lock();
        let mut id = 1_000;
        while guard.0.set_param(id, 0.5) {
            id += 1;
        }
        slot.bypass.set_bypassed(true);
        // The queue is full: this one cannot go out yet.
        slot.sync_own_bypass(&mut guard.0);
    }
    // The block drains the queue; the next one delivers the bypass.
    for _ in 0..3 {
        left(&mut h);
    }
    assert!(
        unsafe { (*state).bypassed },
        "a bypass dropped at the parameter cap must be retried, not lost"
    );
}

/// HOST-07: Mastering's whole-plugin bypass keeps its lookahead latency
/// (`chain.rs` delays its dry path to match), so it declares
/// `IS_BYPASS`, and bypassing it moves no delay line.
#[test]
fn mastering_declares_its_bypass_and_keeps_its_latency() {
    let Some(path) = plugin_binary("resonance-mastering") else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("load mastering");
    let id = bundle.descriptors()[0].id.clone();
    let inst = bundle.create_instance(&id, SR).expect("instantiate mastering");
    let latency = u64::from(inst.latency_samples());
    assert!(latency > 0, "mastering carries lookahead latency");
    let slot = PluginSlot::new(inst);
    assert!(slot.bypass_param.is_some(), "mastering declares CLAP_PARAM_IS_BYPASS");

    // Its latency survives a bypass, so the comp table does not move.
    let mut tracks: TrackMap = Default::default();
    let mastered = Track::new(1, "mastered".into());
    mastered.push_plugin(FX);
    tracks.insert(1, Arc::new(mastered));
    tracks.insert(2, Arc::new(Track::new(2, "plain".into())));
    let delays = |slot: &PluginSlot| {
        let lat = resonance_audio::test_support::slot_latency(latency, slot.host_bypassed());
        compensation_delays(&chain_latencies(&tracks, |_| lat))
    };
    let engaged = delays(&slot);
    slot.bypass.set_bypassed_settled(true);
    assert!(!slot.host_bypassed(), "the host bypasses it through its own parameter");
    assert_eq!(delays(&slot), engaged);
    assert_eq!(engaged.1, vec![(1, 0), (2, latency)]);
}
