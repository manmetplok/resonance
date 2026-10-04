//! Plugin-param automation renders the same at any block size (code
//! review HOST-06).
//!
//! The host used to sample a plugin-param lane once per block, at the
//! block's first frame, and send it at time 0: the plugin's parameter
//! moved in steps of the block size — 128 frames live, 1024 in a bounce —
//! so an exported filter sweep zippered where playback did not, and a
//! bounce or freeze never matched what was heard. Now each lane is
//! sampled on a fixed timeline grid and sent as time-stamped events, so
//! the parameter is one function of the timeline whatever renders it.
//!
//! The null test: one track, a clip, and a sample-accurate fake gain
//! effect whose gain a lane sweeps. The live callback at 128 frames (and
//! at an odd 100) must produce exactly what the offline renderer produces
//! in its 1024-frame chunks.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use clap_sys::events::{clap_event_param_value, CLAP_EVENT_PARAM_VALUE};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    __instance_from_raw_for_test, render_stem, AutomationSnapshot, MixAudioHarness, PluginSlot,
    ResolvedParamLane, StemSource,
};
use resonance_audio::types::*;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

const SR: u32 = 48_000;
const FX: PluginInstanceId = 700;
const GAIN: u32 = 3;
/// 12 bounce chunks.
const FRAMES: usize = 12 * 1024;

/// The fake's gain, kept in its plugin data.
struct Gain(f32);

unsafe extern "C" fn ok(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn noop(_p: *const clap_plugin) {}
unsafe extern "C" fn activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}

/// `out = in * gain`, with each `PARAM_VALUE` applied from its own sample
/// on — what a sample-accurate CLAP plugin does.
unsafe extern "C" fn process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    unsafe {
        let gain = &mut *((*p).plugin_data as *mut Gain);
        let pr = &*process;
        let frames = pr.frames_count as usize;
        let out = &*pr.audio_outputs;
        let chans = [*out.data32, *out.data32.add(1)];
        let events = &*pr.in_events;
        let count = (events.size.unwrap())(events);
        let mut next = 0;
        for i in 0..frames {
            while next < count {
                let h = (events.get.unwrap())(events, next);
                if (*h).time as usize > i {
                    break;
                }
                if (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                    let e = &*(h as *const clap_event_param_value);
                    if e.param_id == GAIN {
                        gain.0 = e.value as f32;
                    }
                }
                next += 1;
            }
            for c in chans {
                *c.add(i) *= gain.0;
            }
        }
    }
    CLAP_PROCESS_CONTINUE
}

fn gain_fx() -> PluginSlot {
    let inst = __instance_from_raw_for_test(
        |_host| {
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: Box::into_raw(Box::new(Gain(1.0))) as *mut c_void,
                init: Some(ok),
                destroy: Some(noop),
                activate: Some(activate),
                deactivate: Some(noop),
                start_processing: Some(ok),
                stop_processing: Some(noop),
                reset: Some(noop),
                process: Some(process),
                get_extension: None,
                on_main_thread: None,
            })) as *const clap_plugin
        },
        SR,
    )
    .expect("fake gain");
    PluginSlot::new(inst)
}

fn clip() -> AudioClip {
    let data: Vec<f32> = (0..FRAMES + 4096)
        .flat_map(|i| {
            let s = (i as f32 * 0.031).sin() * 0.5;
            [s, s]
        })
        .collect();
    AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::memory(data),
        name: "src".into(),
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

/// A gain lane sweeping 0.1 → 1.0 across the render, with a hold and a
/// fall, so there are breakpoints off any block boundary.
fn sweep() -> AutomationSnapshot {
    let lane = AutomationLane::new(
        1,
        AutomationTarget::PluginParam {
            instance: FX,
            param_id: GAIN,
        },
        vec![
            Breakpoint::new(0, 0.1, CurveKind::Linear),
            Breakpoint::new(5_000, 1.0, CurveKind::Linear),
            Breakpoint::new(7_777, 1.0, CurveKind::Linear),
            Breakpoint::new(FRAMES as u64, 0.2, CurveKind::Linear),
        ],
    );
    let mut snap = AutomationSnapshot::default();
    snap.plugin_params.insert(
        FX,
        vec![ResolvedParamLane {
            param_id: GAIN,
            lane,
            min: 0.0,
            max: 1.0,
        }],
    );
    snap
}

fn harness(block: usize, automation: AutomationSnapshot) -> MixAudioHarness {
    let track = Track::new(1, "t".into());
    track.push_plugin(FX);
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![clip()],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        block,
        2,
        SR,
        true,
    );
    h.edit_plugins(|p| p.insert(FX, Arc::new(gain_fx())));
    h.set_automation(automation);
    h
}

/// The live callback, `block` frames at a time, from frame 0.
fn live(block: usize, automation: AutomationSnapshot) -> Vec<f32> {
    let mut h = harness(block, automation);
    h.shared().playing.store(true, Ordering::Relaxed);
    let mut out = Vec::with_capacity(FRAMES * 2);
    while out.len() < FRAMES * 2 {
        out.extend_from_slice(h.render());
    }
    out.truncate(FRAMES * 2);
    out
}

/// The offline renderer (1024-frame chunks), same project.
fn bounce(automation: AutomationSnapshot) -> Vec<f32> {
    let h = harness(128, AutomationSnapshot::default());
    let tempo = Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default()));
    let mut out = render_stem(
        StemSource::Master,
        0,
        FRAMES as u64,
        &h.shared_arc(),
        &tempo,
        &automation,
        SR,
    )
    .expect("stem renders");
    out.truncate(FRAMES * 2);
    out
}

/// Largest sample difference past the first [`SETTLE`] frames: the live
/// callback fades the first block in from a transport start, which the
/// offline renderer does not — nothing to do with automation.
fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .skip(SETTLE * 2)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
}

/// Frames skipped before comparing (see [`max_diff`]).
const SETTLE: usize = 512;

#[test]
fn automated_plugin_param_renders_the_same_live_and_offline() {
    // Control: unautomated, the two renderers agree — so any difference
    // below is the automation's.
    let still_live = live(128, AutomationSnapshot::default());
    let still_bounce = bounce(AutomationSnapshot::default());
    assert!(still_live.iter().any(|s| s.abs() > 0.1), "the render must be audible");
    assert_eq!(max_diff(&still_live, &still_bounce), 0.0, "unautomated renders differ");

    let bounced = bounce(sweep());
    for block in [128, 100] {
        let played = live(block, sweep());
        let d = max_diff(&played, &bounced);
        assert_eq!(
            d, 0.0,
            "automation at {block}-frame blocks differs from the bounce by {d}"
        );
    }
    // Not vacuous: the sweep moved the level.
    assert!(max_diff(&bounced, &still_bounce) > 0.1, "the automation must be audible");
}
