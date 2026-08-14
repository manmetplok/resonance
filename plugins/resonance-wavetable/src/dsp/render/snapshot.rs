//! The once-per-block parameter snapshot.
//!
//! Every atomic parameter the render path reads is loaded exactly once here,
//! at the top of the block, into plain locals. From that point on the
//! per-sample kernel performs zero atomic loads against the shared
//! [`WavetableParams`] and contains no `Param::value()` call at all.

use crate::dsp::filter::FilterType;
use crate::dsp::lfo::LfoShape;
use crate::dsp::modulation::{ModDest, ModSlot, ModSource, NUM_MOD_SLOTS};
use crate::params::WavetableParams;

/// Immutable snapshot of every parameter read by the per-sample render
/// kernel. Built once at the top of each audio block; every field below is
/// a plain local from that point on, so the per-sample loop performs zero
/// atomic loads against the shared [`WavetableParams`].
pub(crate) struct ParamSnapshot {
    pub master_vol: f32,

    pub osc_balance: f32,
    pub osc1_enabled: bool,
    pub osc2_enabled: bool,
    pub osc1_wt: usize,
    pub osc2_wt: usize,
    pub osc1_pos: f32,
    pub osc2_pos: f32,
    pub osc1_coarse: f32,
    pub osc2_coarse: f32,
    pub osc1_fine: f32,
    pub osc2_fine: f32,
    pub osc1_level: f32,
    pub osc2_level: f32,
    pub osc1_pan: f32,
    pub osc2_pan: f32,

    pub filter_enabled: bool,
    pub filter_type: FilterType,
    pub filter_cutoff: f32,
    pub filter_reso: f32,
    pub filter_env_depth: f32,
    pub filter_keytrack: f32,
    pub filter_drive: f32,

    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_sustain: f32,
    pub amp_release: f32,
    pub amp_curve: f32,

    pub mod_attack: f32,
    pub mod_decay: f32,
    pub mod_sustain: f32,
    pub mod_release: f32,
    pub mod_curve: f32,

    pub lfo1_shape: LfoShape,
    pub lfo1_rate: f32,
    pub lfo1_depth: f32,
    pub lfo1_retrigger: bool,

    pub lfo2_shape: LfoShape,
    pub lfo2_rate: f32,
    pub lfo2_depth: f32,
    pub lfo2_retrigger: bool,

    pub lfo3_shape: LfoShape,
    pub lfo3_rate: f32,
    pub lfo3_depth: f32,
    pub lfo3_retrigger: bool,

    pub glide_coeff: f32,
    pub mod_slots: [ModSlot; NUM_MOD_SLOTS],

    pub dist_enabled: bool,
    pub dist_drive: f32,
    pub dist_mix: f32,

    pub chorus_enabled: bool,
    pub chorus_rate: f32,
    pub chorus_depth: f32,
    pub chorus_mix: f32,

    pub delay_enabled: bool,
    pub delay_time_l: f32,
    pub delay_time_r: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
}

impl ParamSnapshot {
    pub(crate) fn capture(params: &WavetableParams, sample_rate: f32) -> Self {
        let glide_enabled = params.glide_enabled.value();
        let glide_time_ms = params.glide_time.value();
        let glide_coeff = if glide_enabled && glide_time_ms > 0.0 {
            1.0 - (-1.0 / (glide_time_ms * 0.001 * sample_rate)).exp()
        } else {
            1.0
        };

        let mod_slots: [ModSlot; NUM_MOD_SLOTS] = std::array::from_fn(|i| ModSlot {
            source: ModSource::from_int(params.mod_slots[i].source.value()),
            dest: ModDest::from_int(params.mod_slots[i].destination.value()),
            amount: params.mod_slots[i].amount.value(),
        });

        Self {
            master_vol: params.master_volume.value(),
            osc_balance: params.osc_balance.value(),
            osc1_enabled: params.osc1.enabled.value(),
            osc2_enabled: params.osc2.enabled.value(),
            osc1_wt: params.osc1.wavetable.value() as usize,
            osc2_wt: params.osc2.wavetable.value() as usize,
            osc1_pos: params.osc1.position.value(),
            osc2_pos: params.osc2.position.value(),
            osc1_coarse: params.osc1.coarse.value() as f32,
            osc2_coarse: params.osc2.coarse.value() as f32,
            osc1_fine: params.osc1.fine.value(),
            osc2_fine: params.osc2.fine.value(),
            osc1_level: params.osc1.level.value(),
            osc2_level: params.osc2.level.value(),
            osc1_pan: params.osc1.pan.value(),
            osc2_pan: params.osc2.pan.value(),

            filter_enabled: params.filter.enabled.value(),
            filter_type: FilterType::from_int(params.filter.filter_type.value()),
            filter_cutoff: params.filter.cutoff.value(),
            filter_reso: params.filter.resonance.value(),
            filter_env_depth: params.filter.env_depth.value(),
            filter_keytrack: params.filter.keytrack.value(),
            filter_drive: params.filter.drive.value(),

            amp_attack: params.amp_env.attack.value(),
            amp_decay: params.amp_env.decay.value(),
            amp_sustain: params.amp_env.sustain.value(),
            amp_release: params.amp_env.release.value(),
            amp_curve: params.amp_env.curve.value(),

            mod_attack: params.mod_env.attack.value(),
            mod_decay: params.mod_env.decay.value(),
            mod_sustain: params.mod_env.sustain.value(),
            mod_release: params.mod_env.release.value(),
            mod_curve: params.mod_env.curve.value(),

            lfo1_shape: LfoShape::from_int(params.lfo1.shape.value()),
            lfo1_rate: params.lfo1.rate.value(),
            lfo1_depth: params.lfo1.depth.value(),
            lfo1_retrigger: params.lfo1.retrigger.value(),

            lfo2_shape: LfoShape::from_int(params.lfo2.shape.value()),
            lfo2_rate: params.lfo2.rate.value(),
            lfo2_depth: params.lfo2.depth.value(),
            lfo2_retrigger: params.lfo2.retrigger.value(),

            lfo3_shape: LfoShape::from_int(params.lfo3.shape.value()),
            lfo3_rate: params.lfo3.rate.value(),
            lfo3_depth: params.lfo3.depth.value(),
            lfo3_retrigger: params.lfo3.retrigger.value(),

            glide_coeff,
            mod_slots,

            dist_enabled: params.distortion.enabled.value(),
            dist_drive: params.distortion.drive.value(),
            dist_mix: params.distortion.mix.value(),

            chorus_enabled: params.chorus.enabled.value(),
            chorus_rate: params.chorus.rate.value(),
            chorus_depth: params.chorus.depth.value(),
            chorus_mix: params.chorus.mix.value(),

            delay_enabled: params.delay.enabled.value(),
            delay_time_l: params.delay.time_l.value(),
            delay_time_r: params.delay.time_r.value(),
            delay_feedback: params.delay.feedback.value(),
            delay_mix: params.delay.mix.value(),
        }
    }
}
