//! The host's non-finite output scrub (`clap_host::process`).
//!
//! One NaN emitted by a third-party plugin latches permanently into
//! every recursive stage downstream of the mix graph — channel-strip
//! filters, sends, meters, sidechain detectors, other plugins' feedback
//! state — so the host zeroes non-finite samples at the plugin-output
//! boundary, inside `process_multi_with_key`, before the buffer re-enters
//! the graph. These tests drive that boundary end-to-end through a
//! hand-rolled fake CLAP plugin (no shared library; same harness as
//! `tests/clap_latency_tracking.rs`) whose `process()` writes whatever
//! pattern the test stages, and assert:
//!
//! - non-finite samples (NaN, +Inf, -Inf) come out as 0.0,
//! - finite samples in the same block are preserved (the slow pass is
//!   per-sample, not whole-buffer),
//! - a clean block — including one whose *sum* overflows to Inf, the
//!   fast path's false positive — passes through bit-untouched.

use std::ffi::c_void;
use std::ptr;

use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::__test_support::{ClapInstance, __instance_from_raw_for_test};

// ---------------------------------------------------------------------------
// Fake plugin
// ---------------------------------------------------------------------------

/// What the fake plugin's `process()` writes over the whole block, per
/// channel, staged by the test between calls.
#[derive(Clone, Copy)]
enum Pattern {
    /// `fill` everywhere, then `poison` at each `(index, value)` entry.
    Poisoned {
        fill: f32,
        poison: [(usize, f32); 2],
    },
    /// A small finite ramp: `out[i] = i as f32 * 1e-3`.
    Ramp,
    /// `f32::MAX` everywhere — every sample finite, but the block's sum
    /// overflows to +Inf, exercising the fast path's false positive.
    MaxEverywhere,
}

struct FakeState {
    pattern: Pattern,
    process_calls: u32,
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    &mut *((*plugin).plugin_data as *mut FakeState)
}

unsafe extern "C" fn fake_init(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_destroy(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_activate(
    _plugin: *const clap_plugin,
    _sample_rate: f64,
    _min_frames: u32,
    _max_frames: u32,
) -> bool {
    true
}

unsafe extern "C" fn fake_deactivate(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_stop_processing(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    let state = fake_state(plugin);
    state.process_calls += 1;
    let frames = (*process).frames_count as usize;
    let out = &*(*process).audio_outputs;
    for ch in 0..out.channel_count as usize {
        let data = *out.data32.add(ch);
        let buf = std::slice::from_raw_parts_mut(data, frames);
        match state.pattern {
            Pattern::Poisoned { fill, poison } => {
                buf.fill(fill);
                for (idx, value) in poison {
                    buf[idx] = value;
                }
            }
            Pattern::Ramp => {
                for (i, s) in buf.iter_mut().enumerate() {
                    *s = i as f32 * 1e-3;
                }
            }
            Pattern::MaxEverywhere => buf.fill(f32::MAX),
        }
    }
    CLAP_PROCESS_CONTINUE
}

/// Build a `ClapInstance` around a fresh fake plugin, plus a raw pointer
/// to its backing state so tests can stage the next block's pattern.
/// Both the plugin struct and the state are intentionally leaked — the
/// instance's `Drop` still dereferences them (stop / deactivate /
/// destroy). No audio-ports extension is declared, so the host defaults
/// to one stereo output port, exactly like a legacy effect.
fn make_instance(pattern: Pattern) -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(FakeState {
                pattern,
                process_calls: 0,
            }));
            state_ptr = state;
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(fake_init),
                destroy: Some(fake_destroy),
                activate: Some(fake_activate),
                deactivate: Some(fake_deactivate),
                start_processing: Some(fake_start_processing),
                stop_processing: Some(fake_stop_processing),
                reset: None,
                process: Some(fake_process),
                get_extension: None,
                on_main_thread: None,
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake plugin instance");
    (instance, state_ptr)
}

const FRAMES: usize = 128;

fn run_block(instance: &mut ClapInstance) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; FRAMES];
    let mut r = vec![0.0f32; FRAMES];
    instance.process(&mut l, &mut r, FRAMES);
    (l, r)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn nan_block_is_scrubbed_and_next_clean_block_passes_untouched() {
    let (mut instance, state) = make_instance(Pattern::Poisoned {
        fill: 0.25,
        poison: [(7, f32::NAN), (13, f32::NEG_INFINITY)],
    });
    let state = unsafe { &mut *state };

    // Block 1: the plugin emits NaN and -Inf. Both must leave the host
    // as 0.0; the finite samples around them must survive.
    let (l, r) = run_block(&mut instance);
    assert_eq!(state.process_calls, 1);
    for buf in [&l, &r] {
        assert!(
            buf.iter().all(|s| s.is_finite()),
            "non-finite sample escaped the scrub"
        );
        assert_eq!(buf[7], 0.0, "NaN sample must be zeroed");
        assert_eq!(buf[13], 0.0, "-Inf sample must be zeroed");
        assert_eq!(buf[0], 0.25, "finite neighbour must be preserved");
        assert_eq!(buf[FRAMES - 1], 0.25, "finite neighbour must be preserved");
    }

    // Block 2: the same instance emits a clean ramp — it must pass
    // through bit-exact, proving the scrub latches nothing.
    state.pattern = Pattern::Ramp;
    let (l, r) = run_block(&mut instance);
    assert_eq!(state.process_calls, 2);
    for buf in [&l, &r] {
        for (i, s) in buf.iter().enumerate() {
            assert_eq!(*s, i as f32 * 1e-3, "clean block altered at frame {i}");
        }
    }
}

#[test]
fn opposing_infinities_are_both_zeroed() {
    // +Inf and -Inf in one block sum to NaN — still non-finite, so the
    // fast path must trip and the slow pass must zero both.
    let (mut instance, _state) = make_instance(Pattern::Poisoned {
        fill: -0.5,
        poison: [(3, f32::INFINITY), (4, f32::NEG_INFINITY)],
    });
    let (l, r) = run_block(&mut instance);
    for buf in [&l, &r] {
        assert_eq!(buf[3], 0.0);
        assert_eq!(buf[4], 0.0);
        assert_eq!(buf[2], -0.5, "finite neighbour must be preserved");
        assert_eq!(buf[5], -0.5, "finite neighbour must be preserved");
        assert!(buf.iter().all(|s| s.is_finite()));
    }
}

#[test]
fn finite_block_whose_sum_overflows_is_left_untouched() {
    // Every sample is f32::MAX — finite — but the block's sum overflows
    // to +Inf, so the fast path false-positives into the slow pass. The
    // slow pass is authoritative: it finds nothing non-finite and must
    // change nothing.
    let (mut instance, _state) = make_instance(Pattern::MaxEverywhere);
    let (l, r) = run_block(&mut instance);
    for buf in [&l, &r] {
        assert!(
            buf.iter().all(|s| *s == f32::MAX),
            "scrub must not alter finite samples on a sum-overflow false positive"
        );
    }
}
