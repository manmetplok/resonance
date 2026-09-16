//! A non-finite sample must never wedge the gate.
//!
//! The gain-reduction envelope and the detector's one-pole high-pass in
//! `dsp.rs` are recursions that never leave NaN once they hold one:
//! without the guards there, a single NaN key sample latched `gr_db` (or
//! the key filter's `prev_out`) and wedged the gate until reset. The
//! host scrubs plugin OUTPUT at the mixer boundary, but the plugin's own
//! state must survive on its own — these are CLAP plugins usable in
//! hosts that don't scrub.
//!
//! The guards must be invisible for finite input. `tests/dsp_golden.rs`
//! pins that bit-exactly against the checked-in fixture; the null test
//! below additionally pins that a poisoned-then-reset DSP renders bit
//! identically to a fresh one.

use resonance_gate::dsp::{GateDsp, GateSettings};

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;

fn settings(key_hpf_hz: f32) -> GateSettings {
    GateSettings {
        threshold_db: -40.0,
        ratio: 8.0,
        attack_ms: 0.1,
        hold_ms: 0.0,
        // Fast, so "recovers" is bounded in a few blocks.
        release_ms: 5.0,
        range_db: 60.0,
        hysteresis_db: 0.0,
        key_hpf_hz,
    }
}

/// Deterministic tone at a constant amplitude.
fn tone(amp: f32, offset: usize, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| ((offset + i) as f32 * 0.05).sin() * amp)
        .collect()
}

/// A block of the values the guards exist for: NaN and both infinities.
fn poison(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| match i % 3 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            _ => f32::NEG_INFINITY,
        })
        .collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// One poisoned INPUT block between clean loud ones: every block after
/// the poisoned one must be finite, the gate must be open again with no
/// residual gain reduction, and it must still close on a quiet tail.
/// Run with the key HPF both bypassed and engaged, since the engaged
/// filter is itself recursive state the poison flows through.
#[test]
fn nan_input_block_recovers_and_the_gate_still_gates() {
    for key_hpf_hz in [0.0, 250.0] {
        let s = settings(key_hpf_hz);
        let mut dsp = GateDsp::new(SR);
        let mut n = 0;

        // Open the gate on clean loud material first.
        for _ in 0..4 {
            let mut l = tone(0.5, n, BLOCK);
            let mut r = tone(0.5, n, BLOCK);
            dsp.process_block(&mut l, &mut r, None, BLOCK, &s);
            n += BLOCK;
        }
        assert!(dsp.last_open, "the gate should be open before the poison");

        // The poisoned block. Its own output is allowed to be
        // non-finite — the host scrubs output — but the STATE must not
        // stay poisoned.
        let mut l = poison(BLOCK);
        let mut r = poison(BLOCK);
        dsp.process_block(&mut l, &mut r, None, BLOCK, &s);

        // Clean loud blocks: finite output from the first one, fully
        // open with ~0 dB reduction within a few.
        let mut last = Vec::new();
        for block in 0..4 {
            let mut l = tone(0.5, n, BLOCK);
            let mut r = tone(0.5, n, BLOCK);
            dsp.process_block(&mut l, &mut r, None, BLOCK, &s);
            n += BLOCK;
            assert!(
                l.iter().chain(r.iter()).all(|x| x.is_finite()),
                "hpf {key_hpf_hz} Hz: non-finite output {} block(s) after the poison",
                block + 1
            );
            last = l;
        }
        assert!(
            dsp.last_open,
            "hpf {key_hpf_hz} Hz: the gate never reopened after the poison"
        );
        assert!(
            dsp.last_gr_db < 0.5,
            "hpf {key_hpf_hz} Hz: residual gain reduction {} dB after recovery",
            dsp.last_gr_db
        );
        assert!(
            peak(&last) > 0.4,
            "hpf {key_hpf_hz} Hz: loud signal no longer passes, peak {}",
            peak(&last)
        );

        // And the state machine must still CLOSE: a quiet tail gets
        // gated, so the poison did not leave the gate stuck open.
        let mut tail = Vec::new();
        for _ in 0..8 {
            let mut l = tone(0.001, n, BLOCK);
            let mut r = tone(0.001, n, BLOCK);
            dsp.process_block(&mut l, &mut r, None, BLOCK, &s);
            n += BLOCK;
            tail = l;
        }
        assert!(
            !dsp.last_open,
            "hpf {key_hpf_hz} Hz: the gate no longer closes on quiet material"
        );
        assert!(
            peak(&tail) < 0.0001,
            "hpf {key_hpf_hz} Hz: quiet material is no longer attenuated, peak {}",
            peak(&tail)
        );
    }
}

/// A poisoned external KEY never reaches the audio path, so with clean
/// input the output must stay finite through the poisoned blocks
/// themselves — the guarded detector reads the bad key as silence (the
/// gate simply closes). Afterwards a clean loud key must open it again.
#[test]
fn nan_key_never_reaches_the_output_and_the_key_recovers() {
    let s = settings(250.0);
    let mut dsp = GateDsp::new(SR);
    let mut n = 0;

    let key_l = poison(BLOCK);
    let key_r = poison(BLOCK);
    for block in 0..4 {
        let mut l = tone(0.25, n, BLOCK);
        let mut r = tone(0.25, n, BLOCK);
        dsp.process_block(&mut l, &mut r, Some((&key_l, &key_r)), BLOCK, &s);
        n += BLOCK;
        assert!(
            l.iter().chain(r.iter()).all(|x| x.is_finite()),
            "a poisoned key leaked non-finite samples into block {block}"
        );
    }
    assert!(
        !dsp.last_open,
        "a poisoned key must read as silence and close the gate"
    );

    // A clean, loud key must still open the gate.
    let key = tone(0.5, 0, BLOCK);
    let mut last = Vec::new();
    for _ in 0..4 {
        let mut l = tone(0.25, n, BLOCK);
        let mut r = tone(0.25, n, BLOCK);
        dsp.process_block(&mut l, &mut r, Some((&key, &key)), BLOCK, &s);
        n += BLOCK;
        last = l;
    }
    assert!(dsp.last_open, "the key path never recovered from the poison");
    assert!(
        peak(&last) > 0.2,
        "the reopened gate is still attenuating, peak {}",
        peak(&last)
    );
}

/// Null test: poisoning the DSP and then `reset()`-ing it must leave no
/// trace — the same finite render, sample for sample, bit for bit. This
/// pins both that `reset` clears every guarded state and that the
/// guards themselves never fire on a finite path.
#[test]
fn finite_render_is_bit_identical_after_poison_and_reset() {
    let s = settings(250.0);

    let render = |dsp: &mut GateDsp| -> Vec<f32> {
        let mut out = Vec::new();
        let mut n = 0;
        for block in 0..12 {
            // Level crosses the threshold across blocks so every gate
            // state is exercised.
            let amp = if block % 3 == 0 { 0.5 } else { 0.001 };
            let mut l = tone(amp, n, BLOCK);
            let mut r = tone(amp * 0.85, n, BLOCK);
            dsp.process_block(&mut l, &mut r, None, BLOCK, &s);
            n += BLOCK;
            out.extend_from_slice(&l);
            out.extend_from_slice(&r);
        }
        out
    };

    let mut fresh = GateDsp::new(SR);
    let clean = render(&mut fresh);

    let mut healed = GateDsp::new(SR);
    let mut l = poison(BLOCK);
    let mut r = poison(BLOCK);
    healed.process_block(&mut l, &mut r, None, BLOCK, &s);
    healed.reset();
    let after = render(&mut healed);

    assert_eq!(clean.len(), after.len());
    for (i, (a, b)) in clean.iter().zip(&after).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "sample {i} differs after poison+reset: {a:?} vs {b:?}"
        );
    }
}
