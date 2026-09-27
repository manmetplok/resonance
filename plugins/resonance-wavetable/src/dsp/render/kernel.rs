//! The per-voice, per-sample render kernel.
//!
//! Everything in this file runs inside the innermost loop of the audio
//! callback, so it obeys three hard rules: no allocation, no locking, and no
//! `Param::value()` — all parameters arrive pre-read in [`ParamSnapshot`] and
//! [`BlockPlan`]. The helpers are free functions over `&mut Voice` rather than
//! methods on `SynthEngine` so that the voice, the wavetable set and the RNG
//! can be borrowed disjointly, and every one is `#[inline]` so the split costs
//! no call in the hot path.
//!
//! The genuinely per-sample work is [`osc_kernel`] — a table read, a phase
//! advance and a pan mix, with no transcendentals and no branches beyond the
//! block-constant enable flags. Everything else in here is gated to control
//! rate (see [`SampleCtx::coeff_tick`]) or to a dirty flag.

use resonance_dsp::{constant_power_pan, SimpleRng};

use crate::dsp::lfo::LfoMode;
use crate::dsp::modulation::{self, ModState};
use crate::dsp::oscillator::{self, midi_to_freq};
use crate::dsp::render::plan::BlockPlan;
use crate::dsp::render::snapshot::ParamSnapshot;
use crate::dsp::voice::{OscSetup, Voice, VoiceState};
use crate::dsp::wavetable::Wavetable;

/// Cents of unison detune a full-scale (±1.0) `ModDest::UnisonDetune`
/// modulation adds — the whole range of the `unison_detune` parameter, so
/// an amount of +1.0 can open a stack from 0 to fully detuned.
const UNISON_DETUNE_MOD_CENTS: f32 = 100.0;

/// The handful of values that change from sample to sample but are shared by
/// every voice in that sample.
pub(crate) struct SampleCtx {
    /// True on the samples where the control-rate grid ticks: the modulation
    /// matrix and the filter coefficients are refreshed only then.
    pub coeff_tick: bool,
    /// True when LFO *shapes* must actually be evaluated this sample — a
    /// control tick, or a sample a voice was triggered on. LFO phases advance
    /// every sample regardless.
    pub lfo_vals_needed: bool,
    /// Global (non-retriggered) LFO values, already scaled by depth. Zero
    /// when `lfo_vals_needed` is false, in which case nothing consumes them.
    pub global_lfo: [f32; 3],
    /// The `ModSource::SampleHold` generator's current value, shared by
    /// every voice this sample. Same zero-when-unneeded contract as
    /// `global_lfo`.
    pub sample_hold_val: f32,
}

/// Render one voice's contribution to this sample.
///
/// Returns `None` when the voice contributes nothing — either it was already
/// idle, or its amplitude envelope just finished releasing and it went idle
/// here. Both cases mirror the `continue` they replaced: the voice's mix
/// contribution is skipped entirely rather than added as zero.
#[inline]
pub(crate) fn render_voice(
    voice: &mut Voice,
    snap: &ParamSnapshot,
    plan: &BlockPlan,
    wavetables: &[Wavetable],
    rng: &mut SimpleRng,
    ctx: &SampleCtx,
) -> Option<(f32, f32)> {
    if voice.state == VoiceState::Idle {
        return None;
    }

    // Portamento.
    voice.current_pitch += (voice.target_pitch - voice.current_pitch) * snap.glide_coeff;

    let (lfo1_val, lfo2_val, lfo3_val) = advance_voice_lfos(voice, snap, rng, ctx);

    // Envelopes — coefficients precomputed at block top.
    let amp_env_val = voice.amp_env.next(&plan.amp_coeffs);
    let mod_env_val = voice.mod_env.next(&plan.mod_coeffs);

    // Voice finished releasing -- go idle and stop mixing.
    if voice.amp_env.is_idle() && voice.state == VoiceState::Releasing {
        voice.state = VoiceState::Idle;
        return None;
    }

    refresh_voice_mods(voice, snap, ctx, [lfo1_val, lfo2_val, lfo3_val], mod_env_val);
    let mods = voice.cached_mods;

    // Render oscillators with unison. Skipped entirely when no oscillator can
    // contribute; `osc_l`/`osc_r` then stay 0.0 and the rest of the voice path
    // runs unchanged on silence.
    let mut osc_l = 0.0f32;
    let mut osc_r = 0.0f32;
    if plan.oscs_active {
        refresh_osc_setups(voice, snap, plan, wavetables, &mods);
        let (l, r) = osc_kernel(voice, snap, plan, wavetables);
        osc_l = l;
        osc_r = r;
    }

    // Filter. Coefficients are refreshed at control rate or immediately when
    // a voice was just triggered.
    if snap.filter_enabled {
        if ctx.coeff_tick || voice.filter_dirty {
            refresh_filter_coeffs(voice, snap, plan, &mods, mod_env_val);
        }
        osc_l = voice.filter_l.process(osc_l, snap.filter_type);
        osc_r = voice.filter_r.process(osc_r, snap.filter_type);
    } else {
        voice.last_filter_cutoff = snap.filter_cutoff;
    }

    let amp = amp_env_val * voice.velocity * (1.0 + mods.amp_level).max(0.0);
    Some((osc_l * amp, osc_r * amp))
}

/// Resolve this voice's three LFO values and advance the per-voice phases.
///
/// A retriggered LFO uses the voice's own phase; a free-running one uses the
/// global value already computed for this sample. The same control-rate gate
/// as the globals applies: the phase always advances, the shape is only
/// evaluated when the mod matrix is about to consume it. Evaluating a shape
/// is a `sin()` for the default sine LFO, so gating it removes up to three
/// transcendental calls per voice per sample.
#[inline]
fn advance_voice_lfos(
    voice: &mut Voice,
    snap: &ParamSnapshot,
    rng: &mut SimpleRng,
    ctx: &SampleCtx,
) -> (f32, f32, f32) {
    // `voice.mod_dirty` is only ever set by `trigger()`, which runs in this
    // sample's event drain, so `lfo_vals_needed` already covers it.
    debug_assert!(
        ctx.lfo_vals_needed || !voice.mod_dirty,
        "mod matrix would consume stale LFO values"
    );

    // Only `Retrig` has a per-voice phase. `Sync` deliberately does not: its
    // whole point is one phase locked to the timeline, which a per-note reset
    // would break.
    let [mut lfo1_val, mut lfo2_val, mut lfo3_val] = ctx.global_lfo;
    if snap.lfo1_mode == LfoMode::Retrig {
        if ctx.lfo_vals_needed {
            lfo1_val = voice.lfo1.value(snap.lfo1_shape) * snap.lfo1_depth;
        }
        voice.lfo1.advance(snap.lfo1_shape, rng);
    }
    if snap.lfo2_mode == LfoMode::Retrig {
        if ctx.lfo_vals_needed {
            lfo2_val = voice.lfo2.value(snap.lfo2_shape) * snap.lfo2_depth;
        }
        voice.lfo2.advance(snap.lfo2_shape, rng);
    }
    if snap.lfo3_mode == LfoMode::Retrig {
        if ctx.lfo_vals_needed {
            lfo3_val = voice.lfo3.value(snap.lfo3_shape) * snap.lfo3_depth;
        }
        voice.lfo3.advance(snap.lfo3_shape, rng);
    }
    (lfo1_val, lfo2_val, lfo3_val)
}

/// Re-evaluate the modulation matrix into the voice's cache, at control rate.
///
/// The slot evaluation is non-trivial (11 destinations x up to
/// `NUM_MOD_SLOTS` branches) and its inputs — LFO values, the mod envelope,
/// key tracking, velocity — are all sub-audio-rate, so it runs at the same
/// control rate as the filter coefficients. `mod_dirty` forces an immediate
/// refresh on freshly-triggered voices.
#[inline]
fn refresh_voice_mods(
    voice: &mut Voice,
    snap: &ParamSnapshot,
    ctx: &SampleCtx,
    lfo_vals: [f32; 3],
    mod_env_val: f32,
) {
    if !(ctx.coeff_tick || voice.mod_dirty) {
        return;
    }
    let fresh = modulation::evaluate_mod_matrix(
        &snap.mod_slots,
        lfo_vals[0],
        lfo_vals[1],
        lfo_vals[2],
        mod_env_val,
        voice.velocity,
        voice.current_pitch,
        voice.random_value,
        ctx.sample_hold_val,
        voice.alternate_value,
    );
    // Only the oscillator-facing destinations invalidate the cached per-unison
    // setup; a filter LFO sweeping every tick must not force an oscillator
    // re-plan.
    if !fresh.osc_setup_eq(&voice.cached_mods) {
        voice.osc_setup_dirty = true;
    }
    voice.cached_mods = fresh;
    voice.mod_dirty = false;
}

/// Rebuild the per-unison [`OscSetup`] caches, but only when one of their
/// inputs moved: the mod matrix produced new oscillator-facing values,
/// portamento shifted the pitch, or the block just started (snapshot params
/// may differ).
///
/// This is where `exp2` (pitch -> Hz), `log2` (mip-level selection) and
/// `sin`/`cos` (constant-power pan) live. Previously all four ran per sample
/// per unison per oscillator. For a static-pitch note they now run once per
/// control tick, and for a patch with no oscillator-facing modulation, once
/// per block.
#[inline]
fn refresh_osc_setups(
    voice: &mut Voice,
    snap: &ParamSnapshot,
    plan: &BlockPlan,
    wavetables: &[Wavetable],
    mods: &ModState,
) {
    if !(voice.osc_setup_dirty || voice.current_pitch != voice.osc_setup_pitch) {
        return;
    }

    // Osc balance and unison detune are modulation destinations, so both are
    // resolved here (control rate, per voice) rather than block-constant.
    // With no routing to either, `mods.osc_balance` / `mods.unison_detune`
    // are 0.0 and these reduce to the block-constant expressions they
    // replaced, term for term.
    let balance = (snap.osc_balance + mods.osc_balance).clamp(-1.0, 1.0);
    let osc1_level = plan.osc1_level * (1.0 - balance.max(0.0));
    let osc2_level = plan.osc2_level * (1.0 - balance.min(0.0).abs());
    // Full-scale modulation sweeps the detune param's whole 0..100 ct range.
    let detune_cents =
        (snap.unison_detune + mods.unison_detune * UNISON_DETUNE_MOD_CENTS).clamp(0.0, 100.0);

    for u in 0..voice.unison_count {
        let sub = &mut voice.unison[u];
        let detune = sub.detune_spread * detune_cents * 0.5 / 100.0;

        if let Some(idx) = plan.wt1_idx {
            let wt = &wavetables[idx];
            let pitch = voice.current_pitch
                + snap.osc1_coarse
                + snap.osc1_fine / 100.0
                + detune
                + mods.osc1_pitch;
            let freq = midi_to_freq(pitch);
            let pos = (snap.osc1_pos + mods.osc1_position).clamp(0.0, 1.0);
            let pan = (snap.osc1_pan + sub.pan_offset + mods.osc1_pan).clamp(-1.0, 1.0);
            let (pan_l, pan_r) = constant_power_pan(pan);
            sub.osc1_setup = OscSetup {
                phase_inc: oscillator::phase_inc(freq, plan.sample_rate),
                tap: oscillator::plan_tap(wt, pos, freq, plan.sample_rate),
                level: osc1_level,
                pan_l,
                pan_r,
            };
        }

        if let Some(idx) = plan.wt2_idx {
            let wt = &wavetables[idx];
            let pitch = voice.current_pitch
                + snap.osc2_coarse
                + snap.osc2_fine / 100.0
                + detune
                + mods.osc2_pitch;
            let freq = midi_to_freq(pitch);
            let pos = (snap.osc2_pos + mods.osc2_position).clamp(0.0, 1.0);
            let pan = (snap.osc2_pan + sub.pan_offset + mods.osc2_pan).clamp(-1.0, 1.0);
            let (pan_l, pan_r) = constant_power_pan(pan);
            sub.osc2_setup = OscSetup {
                phase_inc: oscillator::phase_inc(freq, plan.sample_rate),
                tap: oscillator::plan_tap(wt, pos, freq, plan.sample_rate),
                level: osc2_level,
                pan_l,
                pan_r,
            };
        }
    }

    voice.osc_setup_dirty = false;
    voice.osc_setup_pitch = voice.current_pitch;
}

/// ==== THE PER-SAMPLE KERNEL ====
///
/// Table read, phase advance, pan mix, for every unison sub-voice of both
/// oscillators. No transcendentals, no allocation, and no branches beyond the
/// block-constant enable flags — everything expensive was resolved into
/// [`OscSetup`] by [`refresh_osc_setups`]. Anything added here is paid once
/// per unison per oscillator per sample, so it is the one function in the
/// crate where that cost has to be argued for explicitly.
#[inline]
fn osc_kernel(
    voice: &mut Voice,
    snap: &ParamSnapshot,
    plan: &BlockPlan,
    wavetables: &[Wavetable],
) -> (f32, f32) {
    let mut osc_l = 0.0f32;
    let mut osc_r = 0.0f32;

    let wt1 = plan.wt1_idx.map(|i| &wavetables[i]);
    let wt2 = plan.wt2_idx.map(|i| &wavetables[i]);

    for u in 0..voice.unison_count {
        let sub = &mut voice.unison[u];

        if snap.osc1_enabled {
            if let Some(wt) = wt1 {
                let s = &sub.osc1_setup;
                let sample = oscillator::read_tap(wt, &s.tap, sub.osc1_phase) * s.level;
                osc_l += sample * s.pan_l;
                osc_r += sample * s.pan_r;
                sub.osc1_phase += s.phase_inc;
                sub.osc1_phase -= sub.osc1_phase.floor();
            }
        }

        if snap.osc2_enabled {
            if let Some(wt) = wt2 {
                let s = &sub.osc2_setup;
                let sample = oscillator::read_tap(wt, &s.tap, sub.osc2_phase) * s.level;
                osc_l += sample * s.pan_l;
                osc_r += sample * s.pan_r;
                sub.osc2_phase += s.phase_inc;
                sub.osc2_phase -= sub.osc2_phase.floor();
            }
        }
    }

    let unison_scale = 1.0 / (voice.unison_count as f32).sqrt();
    (osc_l * unison_scale, osc_r * unison_scale)
}

/// Recompute the voice's stereo filter coefficients from cutoff, key
/// tracking, the mod envelope and the mod matrix.
///
/// Called at control rate (`tan()` and the three SVF coefficient divides are
/// the bulk of per-voice filter CPU) or immediately for a freshly triggered
/// voice via `filter_dirty`.
#[inline]
fn refresh_filter_coeffs(
    voice: &mut Voice,
    snap: &ParamSnapshot,
    plan: &BlockPlan,
    mods: &ModState,
    mod_env_val: f32,
) {
    let key_offset = snap.filter_keytrack * (voice.current_pitch - 60.0) / 12.0;
    let env_offset = snap.filter_env_depth * mod_env_val;
    let cutoff =
        snap.filter_cutoff * 2.0f32.powf(key_offset + env_offset * 5.0 + mods.filter_cutoff * 5.0);
    let cutoff = cutoff.clamp(20.0, 20000.0);
    let reso = (snap.filter_reso + mods.filter_resonance).clamp(0.0, 1.0);

    voice
        .filter_l
        .set_coeffs(cutoff, reso, plan.sample_rate, snap.filter_drive);
    voice
        .filter_r
        .set_coeffs(cutoff, reso, plan.sample_rate, snap.filter_drive);
    voice.last_filter_cutoff = cutoff;
    voice.filter_dirty = false;
}
