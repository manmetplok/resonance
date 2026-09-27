//! Block-constant values derived from the parameter snapshot.
//!
//! Everything here is resolved once per block and then read — never
//! recomputed — by the per-sample kernel. Keeping it in one plain `Copy`-ish
//! struct is what lets the kernel take a single `&BlockPlan` instead of a
//! dozen loose arguments.

use resonance_plugin::TempoInfo;

use crate::dsp::analog::{self, DriftCoeffs};
use crate::dsp::engine::SynthEngine;
use crate::dsp::envelope::EnvCoeffs;
use crate::dsp::lfo::TransportPlan;
use crate::dsp::render::snapshot::ParamSnapshot;
use crate::dsp::voice::VoiceState;

pub(crate) struct BlockPlan {
    /// Resolved wavetable slots, or `None` when the snapshot's index is out
    /// of range for the loaded table set.
    pub wt1_idx: Option<usize>,
    pub wt2_idx: Option<usize>,

    /// An oscillator only contributes when it is enabled AND its wavetable
    /// index resolved. With both inactive the per-voice unison mixing loop is
    /// pure overhead, so the kernel skips it wholesale.
    ///
    /// The skip is deliberately scoped to the oscillator mixing only:
    /// envelopes (which drive the `Releasing -> Idle` voice transition), LFO
    /// phases, the mod matrix, and the filter's ring-down of residual state
    /// must all keep advancing exactly as before, so voice lifecycle and
    /// modulation continuity are unaffected by toggling the oscillators.
    pub oscs_active: bool,

    /// Per-oscillator level, constant for the block. Kept as a separate
    /// factor from the pan gains so the per-sample multiply order stays
    /// `sample * level * pan`.
    ///
    /// This is the raw `oscN_level` param: the osc-balance crossfade is
    /// applied per voice in `refresh_osc_setups`, because balance is a
    /// modulation destination and so is not block-constant (ba todo #1323).
    pub osc1_level: f32,
    pub osc2_level: f32,

    /// Envelope exponential coefficients, hoisted out of the per-sample loop.
    /// With 32 voices x 2 envelopes x 48 kHz, leaving the `.exp()` inside
    /// `AdsrEnvelope::next` cost millions of calls per second — and
    /// times+curve are stable for the whole block since they come straight
    /// from [`ParamSnapshot`].
    pub amp_coeffs: EnvCoeffs,
    pub mod_coeffs: EnvCoeffs,

    /// Effective frequency of each LFO for this block. A free or retriggered
    /// LFO gets `lfoN_rate`; a tempo-synced one gets the rate its division
    /// implies at the host's tempo (ba todo #1324).
    pub lfo_rates: [f32; 3],

    /// Analog instability, scaled by the `analog` knob. At the default of 0
    /// `analog_on` is false — the drift walk is never stepped and never
    /// dirties the `OscSetup` cache — and the three spreads are exact zeros,
    /// so the terms they scale add `±0.0` / multiply by `1.0`.
    pub analog_on: bool,
    /// Drift walk `[-1, 1]` to semitones.
    pub drift_semis: f32,
    /// Per-note cutoff spread `[-1, 1]` to octaves.
    pub cutoff_spread_oct: f32,
    /// Per-note level spread `[-1, 1]` to a fraction of the level.
    pub level_spread: f32,
    pub drift: DriftCoeffs,

    pub sample_rate: f32,
}

impl SynthEngine {
    /// Resolve the block-constant plan and push block-rate state (LFO rates,
    /// cache invalidation, the active-voice list) into the engine.
    pub(crate) fn plan_block(
        &mut self,
        snap: &ParamSnapshot,
        tempo: Option<TempoInfo>,
    ) -> BlockPlan {
        // Missing wavetable indices fall back to `None` and silently skip
        // that oscillator's output.
        let wt1_idx = (snap.osc1_wt < self.wavetables.len()).then_some(snap.osc1_wt);
        let wt2_idx = (snap.osc2_wt < self.wavetables.len()).then_some(snap.osc2_wt);
        let oscs_active =
            (snap.osc1_enabled && wt1_idx.is_some()) || (snap.osc2_enabled && wt2_idx.is_some());

        // Resolve the transport once, then each LFO's effective rate: the
        // rate param when free, the division-at-tempo when synced.
        let transport = TransportPlan::resolve(tempo);
        let lfo_rates = [
            transport.lfo_rate_hz(snap.lfo1_mode, snap.lfo1_division, snap.lfo1_rate),
            transport.lfo_rate_hz(snap.lfo2_mode, snap.lfo2_division, snap.lfo2_rate),
            transport.lfo_rate_hz(snap.lfo3_mode, snap.lfo3_division, snap.lfo3_rate),
        ];

        // Global LFO rates only need to be refreshed when the rate param
        // itself changes, but `set_rate` is a single division -- cheap
        // enough to call once per block unconditionally.
        self.global_lfo1.set_rate(lfo_rates[0], self.sample_rate);
        self.global_lfo2.set_rate(lfo_rates[1], self.sample_rate);
        self.global_lfo3.set_rate(lfo_rates[2], self.sample_rate);

        // A synced LFO re-anchors on the song position every block rather
        // than integrating its own phase, so it stays locked through a tempo
        // change or a locate instead of drifting from wherever it happened
        // to be.
        if let Some(p) = transport.lfo_anchor_phase(snap.lfo1_mode, snap.lfo1_division) {
            self.global_lfo1.set_phase(p);
        }
        if let Some(p) = transport.lfo_anchor_phase(snap.lfo2_mode, snap.lfo2_division) {
            self.global_lfo2.set_phase(p);
        }
        if let Some(p) = transport.lfo_anchor_phase(snap.lfo3_mode, snap.lfo3_division) {
            self.global_lfo3.set_phase(p);
        }

        self.refresh_active();
        self.seed_voice_lfo_rates(lfo_rates, true);

        BlockPlan {
            wt1_idx,
            wt2_idx,
            oscs_active,
            osc1_level: snap.osc1_level,
            osc2_level: snap.osc2_level,
            amp_coeffs: EnvCoeffs::for_params(
                snap.amp_attack,
                snap.amp_decay,
                snap.amp_sustain,
                snap.amp_release,
                snap.amp_curve,
                self.sample_rate,
            ),
            mod_coeffs: EnvCoeffs::for_params(
                snap.mod_attack,
                snap.mod_decay,
                snap.mod_sustain,
                snap.mod_release,
                snap.mod_curve,
                self.sample_rate,
            ),
            lfo_rates,
            analog_on: snap.analog > 0.0,
            drift_semis: snap.analog * analog::DRIFT_MAX_CENTS / 100.0,
            cutoff_spread_oct: snap.analog * analog::CUTOFF_SPREAD_OCT,
            level_spread: snap.analog * analog::LEVEL_SPREAD,
            drift: self.drift_coeffs,
            sample_rate: self.sample_rate,
        }
    }

    /// Push this block's LFO rate into every live voice's LFO slots, so the
    /// per-sample `next()` calls only have to advance the phase.
    ///
    /// `invalidate_osc_setup` additionally drops each voice's cached
    /// per-unison [`OscSetup`](crate::dsp::voice::OscSetup). Those caches fold
    /// in block-constant snapshot values (oscillator level, pan, scan
    /// position, tuning), so they must never survive a block boundary — a
    /// parameter edit between blocks has to take effect on the first sample
    /// of the next one. Mid-block (after a note-on) the snapshot has not
    /// moved, so only the rates are reseeded; `trigger()` has already marked
    /// the new voice's cache dirty by itself.
    pub(crate) fn seed_voice_lfo_rates(&mut self, lfo_rates: [f32; 3], invalidate_osc_setup: bool) {
        let sample_rate = self.sample_rate;
        for voice in &mut self.voices {
            if voice.state != VoiceState::Idle {
                voice.lfo1.set_rate(lfo_rates[0], sample_rate);
                voice.lfo2.set_rate(lfo_rates[1], sample_rate);
                voice.lfo3.set_rate(lfo_rates[2], sample_rate);
                if invalidate_osc_setup {
                    voice.osc_setup_dirty = true;
                }
            }
        }
    }
}
