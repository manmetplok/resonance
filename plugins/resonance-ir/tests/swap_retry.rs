//! FU-D1a1: a convolver swap (or a clear) that `SwapFader::try_begin_swap`
//! / `try_begin_clear` cannot admit — every parking slot it could fall
//! back on is taken — is kept and retried on a later block, never dropped
//! on the audio thread.
//!
//! `StereoConvolver` is a concrete type (unlike the amp's `Box<dyn
//! NamInference>`), so there is no test double to instrument with a drop
//! counter without reaching into `resonance-dsp` (read-only for this
//! batch) or `resonance_ir::dsp` itself. Instead this proves the same
//! property end to end: each convolver carries a distinctive direct-tap
//! gain, and once a deferred swap finally lands (`has_pending_swap()` ==
//! false), driving an impulse through the engine shows *that* IR's gain,
//! not an earlier or a default one — i.e. the exact payload that was
//! refused is the one that eventually took over, not a substitute for
//! one silently dropped.
//!
//! Needs `test-internals` (on by default for this crate's own tests via
//! the dev-dependency in `Cargo.toml`): `set_retire_sink_for_test` swaps
//! out the real janitor thread `IrEngine::new()` spawns for a channel the
//! test wedges full itself, so parking fills deterministically instead of
//! racing a live thread that drains almost instantly.

use std::sync::mpsc::sync_channel;

use resonance_ir::dsp::{IrEngine, StereoConvolver, SWAP_FADE_SAMPLES};
use resonance_plugin::{Smoother, SmoothingStyle};

const BLOCK_SIZE: usize = 128;
const SAMPLE_RATE: f32 = 44_100.0;

/// A one-tap IR: `gain` at sample 0, silence after. Distinctive and
/// trivial to recognise in the wet output.
fn make_ir(gain: f32) -> Vec<f32> {
    let mut ir = vec![0.0f32; BLOCK_SIZE];
    ir[0] = gain;
    ir
}

fn unity_smoothers() -> (Smoother, Smoother) {
    let mut dry_wet = Smoother::new(SmoothingStyle::Linear(50.0));
    let mut output_gain = Smoother::new(SmoothingStyle::Linear(50.0));
    dry_wet.set_sample_rate(SAMPLE_RATE);
    output_gain.set_sample_rate(SAMPLE_RATE);
    dry_wet.reset(1.0); // fully wet: only the convolver's own gain shows up
    output_gain.reset(1.0);
    (dry_wet, output_gain)
}

/// Runs `frames` of silence (except an impulse on the very first sample
/// of the very first call) through `engine`, returning the wet peak
/// magnitude observed. Ticks the swap crossfade along the way.
fn run_impulse(engine: &mut IrEngine, frames: usize, impulse: bool) -> f32 {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    if impulse {
        l[0] = 1.0;
        r[0] = 1.0;
    }
    let (mut dw, mut og) = unity_smoothers();
    engine.process_block(&mut l, &mut r, &mut dw, &mut og);
    l.iter().chain(r.iter()).fold(0.0f32, |m, &s| m.max(s.abs()))
}

#[test]
fn a_refused_swap_is_deferred_and_lands_once_parking_frees() {
    let mut engine = IrEngine::new(BLOCK_SIZE);

    // Capacity 1, pre-filled: every later `try_send` fails, exactly like
    // resonance-dsp's own DSP2-15 test wedges a dead janitor
    // (resonance-dsp/tests/swap_fader.rs). `_rx` lives for the whole test.
    let (wedge_tx, _rx) = sync_channel(1);
    wedge_tx
        .try_send(StereoConvolver::new(&make_ir(999.0), None, BLOCK_SIZE))
        .expect("wedge the channel");
    engine.set_retire_sink_for_test(wedge_tx);

    engine.install(StereoConvolver::new(&make_ir(0.0), None, BLOCK_SIZE));

    // Every further swap displaces the previous active convolver into
    // retirement; the wedged channel forces it into a parking slot. Once
    // all 4 slots are occupied, the fader must refuse rather than drop.
    let settle_frames = 2 * SWAP_FADE_SAMPLES as usize;
    let mut refused_gain = None;
    for id in 1..20u32 {
        let gain = id as f32;
        engine.begin_swap(StereoConvolver::new(&make_ir(gain), None, BLOCK_SIZE));
        run_impulse(&mut engine, settle_frames, false);
        if engine.has_pending_swap() {
            refused_gain = Some(gain);
            break;
        }
    }
    let refused_gain = refused_gain.expect("a swap is eventually refused once parking fills");

    // Stuck: retrying while parking is still full (nothing drains it
    // here — no owner sweep, matching production minus the janitor this
    // test wedged shut) must keep refusing, not substitute a drop.
    for _ in 0..5 {
        engine.retry_pending();
        assert!(engine.has_pending_swap(), "still stuck: parking was never freed");
    }

    // Free the parking slots the way a real owner would: swap in a sink
    // with room, which `can_admit`'s `flush_parked` drains them into
    // immediately, then let the deferred swap actually land.
    let (tx2, rx2) = sync_channel(8);
    engine.set_retire_sink_for_test(tx2);
    std::thread::spawn(move || while rx2.recv().is_ok() {});

    let mut landed = false;
    for _ in 0..64 {
        engine.retry_pending();
        run_impulse(&mut engine, settle_frames, false);
        if !engine.has_pending_swap() {
            landed = true;
            break;
        }
    }
    assert!(landed, "deferred swap (gain {refused_gain}) never landed after parking freed");

    // Flush any crossfade tail, then confirm the convolver that landed is
    // the one that was refused — not a default or an earlier one, i.e.
    // nothing was lost while it waited.
    run_impulse(&mut engine, settle_frames, false);
    let peak = run_impulse(&mut engine, BLOCK_SIZE * 2, true);
    assert!(
        (peak - refused_gain).abs() < 1.0e-3,
        "expected the deferred convolver's gain {refused_gain}, measured peak {peak}"
    );
}

#[test]
fn a_refused_clear_is_deferred_and_lands_once_parking_frees() {
    let mut engine = IrEngine::new(BLOCK_SIZE);

    let (wedge_tx, _rx) = sync_channel(1);
    wedge_tx
        .try_send(StereoConvolver::new(&make_ir(999.0), None, BLOCK_SIZE))
        .expect("wedge the channel");
    engine.set_retire_sink_for_test(wedge_tx);

    engine.install(StereoConvolver::new(&make_ir(1.0), None, BLOCK_SIZE));

    let settle_frames = 2 * SWAP_FADE_SAMPLES as usize;
    // Fill parking with swaps first (same as above), then ask for a clear.
    let mut parking_full = false;
    for id in 1..20u32 {
        engine.begin_swap(StereoConvolver::new(&make_ir(id as f32), None, BLOCK_SIZE));
        run_impulse(&mut engine, settle_frames, false);
        if engine.has_pending_swap() {
            parking_full = true;
            break;
        }
    }
    assert!(parking_full, "setup: parking never filled");

    engine.begin_clear();
    // The clear request carries no payload, but must still be remembered
    // rather than silently discarded while parking is full.
    for _ in 0..5 {
        engine.retry_pending();
    }

    let (tx2, rx2) = sync_channel(8);
    engine.set_retire_sink_for_test(tx2);
    std::thread::spawn(move || while rx2.recv().is_ok() {});

    let mut landed = false;
    for _ in 0..64 {
        engine.retry_pending();
        run_impulse(&mut engine, settle_frames, false);
        if !engine.has_pending_swap() {
            landed = true;
            break;
        }
    }
    assert!(landed, "deferred swap never landed after parking freed");

    // Once the swap retry lands, the clear requested in the meantime must
    // still take effect — the engine ends up dry (passing the impulse
    // through at unity, `process_block`'s `None` branch), not stuck on
    // whatever the swap retry installed (which would show as some
    // convolver's distinctive — and here, much larger than 1.0 — gain).
    run_impulse(&mut engine, settle_frames, false);
    let peak = run_impulse(&mut engine, BLOCK_SIZE * 4, true);
    assert!(
        (peak - 1.0).abs() < 1.0e-3,
        "expected the deferred clear to land (dry passthrough, peak 1.0), measured peak {peak}"
    );
}
