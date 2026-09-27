//! The oscillator-character variant of the per-sample kernel: oscillator
//! interaction (FM, ring, hard sync), phase warp, and the sub and noise
//! sources.
//!
//! A patch reaches [`osc_kernel`] only when it selects an interaction mode
//! or a warp (see `CharacterPlan::kernel`); everything else keeps running
//! the original kernel in `kernel.rs`, so none of this costs the default
//! path anything but one block-constant branch. The same hard rules apply
//! as there — no allocation, no locks, no transcendental math per sample:
//! every `exp2`/`log2` a warp or a mip bias needs was resolved into the
//! [`OscSetup`] at control rate.
//!
//! # Band-limiting
//!
//! Three things here put energy above what the selected mip level holds:
//!
//! * **Warp and phase modulation** sweep the table faster than the
//!   fundamental. That is handled at plan time by selecting the mip level
//!   for the worst-case sweep rate (`plan_biased_tap` in `kernel.rs`), so
//!   the stretched partials stay under Nyquist.
//! * **Hard sync, and the Mirror/Formant warps**, put a step discontinuity
//!   in the waveform once per period, and **Quantize** once per step. A step
//!   has a 1/f spectrum no table can band-limit, so each one is corrected
//!   with a two-sample polyBLEP ([`blep_split`]). A sync reset also breaks
//!   the slope, which gets the matching polyBLAMP ([`blamp_split`]).
//!
//! That is the usual cheap-and-causal trade-off. Every discontinuity here is
//! predictable one sample ahead from phase and increment, so the correction
//! costs a few multiplies (plus two table reads per sync reset) and no
//! latency. Measured at C6 (`tests/osc_character.rs`), it takes hard sync
//! 12-15 dB below the naive reset. It is weakest exactly where aliasing
//! lives at a high note: a two-sample kernel barely attenuates what folds
//! from just above Nyquist. And a slave whose own edge sits at phase 0 —
//! the bundled saw — is reset mid-edge, cutting the table's band-limited
//! transition in half, which neither correction models. A minBLEP table or
//! oversampling would go further at several times the cost; a crossfaded
//! reset would be cheaper but softens the very edge that makes sync sound
//! like sync.
//!
//! The corrections track each oscillator's own phase increment. Osc1's read
//! phase under phase modulation no longer advances by that increment, so
//! osc1's own warp discontinuities are left uncorrected in FM mode.

use resonance_dsp::SimpleRng;

use crate::dsp::osc_mix::OscMixMode;
use crate::dsp::oscillator::{self, TableTap};
use crate::dsp::render::plan::BlockPlan;
use crate::dsp::render::snapshot::ParamSnapshot;
use crate::dsp::voice::{OscSetup, Voice};
use crate::dsp::warp::{blamp_split, blep_split, Warp};
use crate::dsp::wavetable::Wavetable;

/// Precompute the per-setup constants the per-sample corrections read: the
/// warped value and per-sample slope at phase 0 (where a synced slave
/// restarts) and the height of a wrap-jumping warp's once-per-period step.
/// Runs at control rate, after the tap has been planned.
#[inline]
pub(super) fn finish_setup(wt: &Wavetable, setup: &mut OscSetup) {
    setup.zero_value = warped_read(wt, &setup.tap, &setup.warp, 0.0);
    setup.zero_slope =
        warped_read(wt, &setup.tap, &setup.warp, setup.phase_inc) - setup.zero_value;
    setup.wrap_jump = if setup.warp.jumps_at_wrap() {
        setup.zero_value - warped_read(wt, &setup.tap, &setup.warp, 1.0)
    } else {
        0.0
    };
}

/// One table read at oscillator phase `p` through the warp. With the warp
/// off this is exactly [`oscillator::read_tap`] at `p`.
#[inline]
fn warped_read(wt: &Wavetable, tap: &TableTap, warp: &Warp, p: f64) -> f32 {
    if warp.is_off() {
        return oscillator::read_tap(wt, tap, p);
    }
    let (q, gain) = warp.apply(p);
    let v = oscillator::read_tap(wt, tap, q);
    if gain == 1.0 {
        v
    } else {
        v * gain
    }
}

/// Read one oscillator sample at `phase`, including any polyBLEP residual
/// owed to it by the previous sample (`carry`) and — when `predict` is set
/// — the correction for a discontinuity of its own warp that falls before
/// the next sample.
#[inline]
fn read_osc(wt: &Wavetable, s: &OscSetup, phase: f64, carry: &mut f32, predict: bool) -> f32 {
    // Consume what the previous sample owes this one before this sample
    // stores what it owes the next.
    let owed = std::mem::take(carry);
    let naive = warped_read(wt, &s.tap, &s.warp, phase);
    let mut v = naive;

    if predict && !s.warp.is_off() {
        let inc = s.phase_inc;
        let next = phase + inc;
        if s.warp.jumps_at_wrap() {
            if next >= 1.0 {
                let x = ((next - 1.0) / inc) as f32;
                let (now, later) = blep_split(s.wrap_jump, x);
                v += now;
                *carry += later;
            }
        } else if let Some(n) = s.warp.steps() {
            // More than one step per sample is below the sample grid: there
            // is no single step to correct, and the staircase is then just a
            // coarser read of the table.
            if inc * n < 1.0 {
                let q = (phase * n).floor();
                let boundary = ((q + 1.0) / n).min(1.0);
                if next >= boundary {
                    let x = ((next - boundary) / inc) as f32;
                    // Read mid-step: `boundary * n` can round to just under
                    // the next integer and land back on the current step.
                    let start = if boundary >= 1.0 { 0.0 } else { boundary };
                    let after = start + 0.5 / n;
                    let h = warped_read(wt, &s.tap, &s.warp, after) - naive;
                    let (now, later) = blep_split(h, x);
                    v += now;
                    *carry += later;
                }
            }
        }
    }

    // Zero unless a discontinuity was predicted last sample, so the
    // unwarped read passes through untouched.
    if owed != 0.0 {
        v += owed;
    }
    v
}

/// ==== THE INTERACTION / WARP KERNEL ====
///
/// Same shape as `kernel::osc_kernel` — table read, phase advance, pan mix
/// per unison sub-voice — with osc2 read first so it can modulate, ring or
/// be restarted by osc1. The mixing order (osc1's contribution, then
/// osc2's) and every per-oscillator product are those of the default
/// kernel, so a warp at amount zero, FM or ring at amount zero, all render
/// the default kernel's samples exactly.
#[inline]
pub(super) fn osc_kernel(
    voice: &mut Voice,
    snap: &ParamSnapshot,
    plan: &BlockPlan,
    wavetables: &[Wavetable],
) -> (f32, f32) {
    let mut osc_l = 0.0f32;
    let mut osc_r = 0.0f32;

    let wt1 = plan.wt1_idx.map(|i| &wavetables[i]);
    let wt2 = plan.wt2_idx.map(|i| &wavetables[i]);

    let mode = plan.character.mix_mode;
    let fm = mode == OscMixMode::Fm;
    let ring = mode == OscMixMode::Ring;
    // Sync needs a master: without osc1's table there is no osc1 phase.
    let sync = mode == OscMixMode::Sync && wt1.is_some();
    // Osc2 runs when it is heard or when osc1 needs it.
    let osc2_runs = snap.osc2_enabled || mode.drives_osc2();
    let mix = voice.mix_setup;

    for u in 0..voice.unison_count {
        let sub = &mut voice.unison[u];

        // ---- osc2: heard, modulator, or sync slave ----
        let mut v2 = 0.0f32;
        let mut slave_restart: Option<f64> = None;
        if let (Some(wt), true) = (wt2, osc2_runs) {
            let s = &sub.osc2_setup;
            v2 = read_osc(wt, s, sub.osc2_phase, &mut sub.osc2_carry, true);

            if sync {
                // Will the master wrap before the next sample? Its phase
                // increment is constant over the sample, so the reset time
                // is exact: `x` samples before the next one.
                let inc1 = sub.osc1_setup.phase_inc;
                let next1 = sub.osc1_phase + inc1;
                if next1 >= 1.0 {
                    let x = ((next1 - 1.0) / inc1).clamp(0.0, 1.0);
                    // Where the slave is at the reset, the step from there
                    // to its restart value, and the change of slope (both
                    // as per-sample differences, so a table's own steep
                    // edge at phase 0 counts as the sampled signal sees
                    // it). Both are corrected: a reset rarely lands where
                    // the wave is flat, and the slope break is most of
                    // what the step correction alone leaves behind.
                    let at = sub.osc2_phase + s.phase_inc * (1.0 - x);
                    let at = at - at.floor();
                    let before = at - s.phase_inc;
                    let before = before - before.floor();
                    let w_at = warped_read(wt, &s.tap, &s.warp, at);
                    let w_before = warped_read(wt, &s.tap, &s.warp, before);
                    let h = s.zero_value - w_at;
                    let d = s.zero_slope - (w_at - w_before);
                    let (step_now, step_later) = blep_split(h, x as f32);
                    let (bend_now, bend_later) = blamp_split(d, x as f32);
                    v2 += step_now + bend_now;
                    sub.osc2_carry += step_later + bend_later;
                    // The slave restarts at the reset instant, so by the
                    // next sample it has already run for `x` samples.
                    slave_restart = Some(x * s.phase_inc);
                }
            }
        }

        // ---- osc1: carrier / master ----
        if let Some(wt) = wt1 {
            let s = &sub.osc1_setup;
            if snap.osc1_enabled {
                let phase = if fm {
                    // Phase modulation: offset the read phase, never the
                    // accumulator (see `osc_mix::PM_DEPTH_CYCLES`).
                    let p = sub.osc1_phase + mix.pm_depth * v2 as f64;
                    p - p.floor()
                } else {
                    sub.osc1_phase
                };
                let mut v1 = read_osc(wt, s, phase, &mut sub.osc1_carry, !fm);
                if ring {
                    v1 *= (1.0 - mix.ring_wet) + mix.ring_wet * v2;
                }
                let sample = v1 * s.level;
                osc_l += sample * s.pan_l;
                osc_r += sample * s.pan_r;
            }
            // A muted master still clocks its slave.
            if snap.osc1_enabled || sync {
                sub.osc1_phase += s.phase_inc;
                sub.osc1_phase -= sub.osc1_phase.floor();
            }
        }

        // ---- osc2 mix and advance ----
        if let (Some(_), true) = (wt2, osc2_runs) {
            let s = &sub.osc2_setup;
            if snap.osc2_enabled {
                let sample = v2 * s.level;
                osc_l += sample * s.pan_l;
                osc_r += sample * s.pan_r;
            }
            match slave_restart {
                Some(p) => sub.osc2_phase = p,
                None => {
                    sub.osc2_phase += s.phase_inc;
                    sub.osc2_phase -= sub.osc2_phase.floor();
                }
            }
        }
    }

    let unison_scale = 1.0 / (voice.unison_count as f32).sqrt();
    (osc_l * unison_scale, osc_r * unison_scale)
}

/// One sample of the sub oscillator and the noise source, summed at their
/// levels. Only called when at least one of them is non-zero.
#[inline]
pub(super) fn sub_noise(voice: &mut Voice, plan: &BlockPlan, rng: &mut SimpleRng) -> f32 {
    let ch = &plan.character;
    let mut out = 0.0f32;
    if ch.sub_active {
        out += voice.sub.next(ch.sub_wave, voice.sub_inc) * ch.sub_level;
    }
    if ch.noise_active {
        let n = voice
            .noise
            .next(rng, ch.noise_type, ch.noise_tilt_coeff, ch.noise_color);
        out += n * ch.noise_level;
    }
    out
}
