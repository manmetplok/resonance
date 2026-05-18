//! Block-rate audio rendering for the wavetable engine.
//!
//! The hot path snapshots every atomic parameter once at block start into
//! [`ParamSnapshot`], then runs a tight per-sample loop using those locals.
//! This avoids the multi-million atomic loads per second a naive
//! render-per-frame design would otherwise perform, and keeps the per-sample
//! kernel free of `Param::value()` calls entirely.
//!
//! Filter coefficients are refreshed at control rate (every
//! [`FILTER_COEFF_INTERVAL`] samples) since the filter's modulation sources
//! -- LFOs, envelopes, key tracking -- are all sub-audio-rate. Freshly
//! triggered voices force an immediate coefficient refresh via
//! `Voice::filter_dirty`.

use resonance_dsp::constant_power_pan;
use resonance_plugin::{EventIterator, NoteEvent};

use crate::effects::Distortion;
use crate::engine::SynthEngine;
use crate::envelope::EnvCoeffs;
use crate::filter::FilterType;
use crate::lfo::LfoShape;
use crate::modulation::{self, ModDest, ModSlot, ModSource, NUM_MOD_SLOTS};
use crate::oscillator::{self, midi_to_freq, read_wavetable};
use crate::params::WavetableParams;
use crate::voice::VoiceState;

/// Update filter coefficients every N samples. `tan()` and the three SVF
/// coefficient divides are the bulk of per-voice filter CPU, and modulation
/// sources top out well below sample rate / this interval (~3 kHz at 48 kHz),
/// so stair-stepping here is acoustically transparent for any realistic LFO
/// or envelope sweep.
const FILTER_COEFF_INTERVAL: u32 = 16;

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
    fn capture(params: &WavetableParams, sample_rate: f32) -> Self {
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

impl SynthEngine {
    /// Render a full stereo block into `left` / `right`, draining MIDI events
    /// with sample-accurate timing. Replaces the old `render_frame`-per-sample
    /// entry point; all atomic parameter loads happen once up front rather
    /// than per sample.
    pub fn render_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        params: &WavetableParams,
        events: &mut EventIterator<'_>,
    ) {
        let snap = ParamSnapshot::capture(params, self.sample_rate);

        // Resolve wavetable references once per block. Missing indices fall
        // back to `None` and silently skip oscillator output.
        let wt1_idx = if snap.osc1_wt < self.wavetables.len() {
            Some(snap.osc1_wt)
        } else {
            None
        };
        let wt2_idx = if snap.osc2_wt < self.wavetables.len() {
            Some(snap.osc2_wt)
        } else {
            None
        };

        // Global LFO rates only need to be refreshed when the rate param
        // itself changes, but `set_rate` is a single division -- cheap
        // enough to call once per block unconditionally.
        self.global_lfo1.set_rate(snap.lfo1_rate, self.sample_rate);
        self.global_lfo2.set_rate(snap.lfo2_rate, self.sample_rate);
        self.global_lfo3.set_rate(snap.lfo3_rate, self.sample_rate);

        // Push the same rate into every voice's LFO slot once. The
        // per-sample `next()` calls then just advance the phase.
        for voice in &mut self.voices {
            if voice.state != VoiceState::Idle {
                voice.lfo1.set_rate(snap.lfo1_rate, self.sample_rate);
                voice.lfo2.set_rate(snap.lfo2_rate, self.sample_rate);
                voice.lfo3.set_rate(snap.lfo3_rate, self.sample_rate);
            }
        }

        // Hoist envelope exponential coefficients out of the per-sample
        // loop. With 32 voices × 2 envelopes × 48 kHz, leaving the
        // `.exp()` inside `AdsrEnvelope::next` cost millions of calls
        // per second — and times+curve are stable for the whole block
        // since they come straight from `ParamSnapshot`.
        let amp_coeffs = EnvCoeffs::for_params(
            snap.amp_attack,
            snap.amp_decay,
            snap.amp_sustain,
            snap.amp_release,
            snap.amp_curve,
            self.sample_rate,
        );
        let mod_coeffs = EnvCoeffs::for_params(
            snap.mod_attack,
            snap.mod_decay,
            snap.mod_sustain,
            snap.mod_release,
            snap.mod_curve,
            self.sample_rate,
        );

        let mut next_event = events.next_event();
        let sample_rate = self.sample_rate;

        for sample_id in 0..frames {
            // Drain any events whose timing landed on this sample. Note
            // events mutate voice state but never parameters, so the
            // snapshot above remains valid for the rest of the block.
            while let Some(ref event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.note_on(*note, *velocity, params);
                        // The freshly triggered voice also needs its LFO
                        // rates seeded for this block.
                        for voice in &mut self.voices {
                            if voice.state != VoiceState::Idle {
                                voice.lfo1.set_rate(snap.lfo1_rate, sample_rate);
                                voice.lfo2.set_rate(snap.lfo2_rate, sample_rate);
                                voice.lfo3.set_rate(snap.lfo3_rate, sample_rate);
                            }
                        }
                    }
                    NoteEvent::NoteOff { note, .. } => {
                        self.note_off(*note);
                    }
                    NoteEvent::Choke { note, .. } => {
                        self.note_off(*note);
                    }
                }
                next_event = events.next_event();
            }

            // Advance global LFOs. Cheap: four adds + one wrap.
            let global_lfo1_val =
                self.global_lfo1.next(snap.lfo1_shape, &mut self.rng) * snap.lfo1_depth;
            let global_lfo2_val =
                self.global_lfo2.next(snap.lfo2_shape, &mut self.rng) * snap.lfo2_depth;
            let global_lfo3_val =
                self.global_lfo3.next(snap.lfo3_shape, &mut self.rng) * snap.lfo3_depth;

            // Sample index within the control-rate grid. Using a power-of-
            // two interval lets the compiler compile the masks as a cheap
            // AND rather than a divmod.
            let coeff_tick = (sample_id as u32 & (FILTER_COEFF_INTERVAL - 1)) == 0;

            let mut mix_l = 0.0f32;
            let mut mix_r = 0.0f32;

            for voice in &mut self.voices {
                if voice.state == VoiceState::Idle {
                    continue;
                }

                // Portamento
                voice.current_pitch +=
                    (voice.target_pitch - voice.current_pitch) * snap.glide_coeff;

                // LFO values (per-voice or global)
                let lfo1_val = if snap.lfo1_retrigger {
                    voice.lfo1.next(snap.lfo1_shape, &mut self.rng) * snap.lfo1_depth
                } else {
                    global_lfo1_val
                };
                let lfo2_val = if snap.lfo2_retrigger {
                    voice.lfo2.next(snap.lfo2_shape, &mut self.rng) * snap.lfo2_depth
                } else {
                    global_lfo2_val
                };
                let lfo3_val = if snap.lfo3_retrigger {
                    voice.lfo3.next(snap.lfo3_shape, &mut self.rng) * snap.lfo3_depth
                } else {
                    global_lfo3_val
                };

                // Envelopes — coefficients precomputed at block top.
                let amp_env_val = voice.amp_env.next(&amp_coeffs);
                let mod_env_val = voice.mod_env.next(&mod_coeffs);

                // Voice finished releasing -- go idle and stop mixing.
                if voice.amp_env.is_idle() && voice.state == VoiceState::Releasing {
                    voice.state = VoiceState::Idle;
                    continue;
                }

                // Modulation matrix. The slot evaluation is non-trivial
                // (11 dests × up to NUM_MOD_SLOTS branches) and its
                // inputs -- LFO values, the mod envelope, key tracking,
                // velocity -- are all sub-audio-rate, so we evaluate at
                // the same control rate as the filter coefficients and
                // cache on the voice. `mod_dirty` forces an immediate
                // refresh on freshly-triggered voices.
                if coeff_tick || voice.mod_dirty {
                    voice.cached_mods = modulation::evaluate_mod_matrix(
                        &snap.mod_slots,
                        lfo1_val,
                        lfo2_val,
                        lfo3_val,
                        mod_env_val,
                        voice.velocity,
                        voice.current_pitch,
                    );
                    voice.mod_dirty = false;
                }
                let mods = voice.cached_mods;

                // Render oscillators with unison
                let mut osc_l = 0.0f32;
                let mut osc_r = 0.0f32;

                for u in 0..voice.unison_count {
                    let sub = &mut voice.unison[u];

                    if snap.osc1_enabled {
                        if let Some(idx) = wt1_idx {
                            let wt = &self.wavetables[idx];
                            let pitch = voice.current_pitch
                                + snap.osc1_coarse
                                + snap.osc1_fine / 100.0
                                + sub.detune_cents / 100.0
                                + mods.osc1_pitch;
                            let freq = midi_to_freq(pitch);
                            let pos = (snap.osc1_pos + mods.osc1_position).clamp(0.0, 1.0);
                            let sample = read_wavetable(wt, sub.osc1_phase, pos, freq);
                            sub.osc1_phase += oscillator::phase_inc(freq, sample_rate);
                            sub.osc1_phase -= sub.osc1_phase.floor();

                            let pan =
                                (snap.osc1_pan + sub.pan_offset + mods.osc1_pan).clamp(-1.0, 1.0);
                            let (pl, pr) = constant_power_pan(pan);
                            let level = snap.osc1_level * (1.0 - snap.osc_balance.max(0.0));
                            osc_l += sample * level * pl;
                            osc_r += sample * level * pr;
                        }
                    }

                    if snap.osc2_enabled {
                        if let Some(idx) = wt2_idx {
                            let wt = &self.wavetables[idx];
                            let pitch = voice.current_pitch
                                + snap.osc2_coarse
                                + snap.osc2_fine / 100.0
                                + sub.detune_cents / 100.0
                                + mods.osc2_pitch;
                            let freq = midi_to_freq(pitch);
                            let pos = (snap.osc2_pos + mods.osc2_position).clamp(0.0, 1.0);
                            let sample = read_wavetable(wt, sub.osc2_phase, pos, freq);
                            sub.osc2_phase += oscillator::phase_inc(freq, sample_rate);
                            sub.osc2_phase -= sub.osc2_phase.floor();

                            let pan =
                                (snap.osc2_pan + sub.pan_offset + mods.osc2_pan).clamp(-1.0, 1.0);
                            let (pl, pr) = constant_power_pan(pan);
                            let level = snap.osc2_level * (1.0 - snap.osc_balance.min(0.0).abs());
                            osc_l += sample * level * pl;
                            osc_r += sample * level * pr;
                        }
                    }
                }

                let unison_scale = 1.0 / (voice.unison_count as f32).sqrt();
                osc_l *= unison_scale;
                osc_r *= unison_scale;

                // Filter. Coefficients are refreshed at control rate or
                // immediately when a voice was just triggered.
                if snap.filter_enabled {
                    if coeff_tick || voice.filter_dirty {
                        let key_offset = snap.filter_keytrack * (voice.current_pitch - 60.0) / 12.0;
                        let env_offset = snap.filter_env_depth * mod_env_val;
                        let cutoff = snap.filter_cutoff
                            * 2.0f32.powf(key_offset + env_offset * 5.0 + mods.filter_cutoff * 5.0);
                        let cutoff = cutoff.clamp(20.0, 20000.0);
                        let reso = (snap.filter_reso + mods.filter_resonance).clamp(0.0, 1.0);

                        voice
                            .filter_l
                            .set_coeffs(cutoff, reso, sample_rate, snap.filter_drive);
                        voice
                            .filter_r
                            .set_coeffs(cutoff, reso, sample_rate, snap.filter_drive);
                        voice.last_filter_cutoff = cutoff;
                        voice.filter_dirty = false;
                    }
                    osc_l = voice.filter_l.process(osc_l, snap.filter_type);
                    osc_r = voice.filter_r.process(osc_r, snap.filter_type);
                } else {
                    voice.last_filter_cutoff = snap.filter_cutoff;
                }

                // Cache post-modulation osc positions + LFO phases for viz.
                voice.last_osc1_pos = (snap.osc1_pos + mods.osc1_position).clamp(0.0, 1.0);
                voice.last_osc2_pos = (snap.osc2_pos + mods.osc2_position).clamp(0.0, 1.0);
                voice.last_lfo_phases[0] = if snap.lfo1_retrigger {
                    voice.lfo1.phase
                } else {
                    self.global_lfo1.phase
                };
                voice.last_lfo_phases[1] = if snap.lfo2_retrigger {
                    voice.lfo2.phase
                } else {
                    self.global_lfo2.phase
                };
                voice.last_lfo_phases[2] = if snap.lfo3_retrigger {
                    voice.lfo3.phase
                } else {
                    self.global_lfo3.phase
                };

                let amp = amp_env_val * voice.velocity * (1.0 + mods.amp_level).max(0.0);
                mix_l += osc_l * amp;
                mix_r += osc_r * amp;
            }

            // Effects chain. Parameter values are already snapshotted; the
            // enable flags are the same for the whole block so the
            // `if`-chains predict perfectly.
            if snap.dist_enabled {
                let (dl, dr) = Distortion::process(mix_l, mix_r, snap.dist_drive, snap.dist_mix);
                mix_l = dl;
                mix_r = dr;
            }

            if snap.chorus_enabled {
                let (cl, cr) = self.chorus.process(
                    mix_l,
                    mix_r,
                    snap.chorus_rate,
                    snap.chorus_depth,
                    snap.chorus_mix,
                );
                mix_l = cl;
                mix_r = cr;
            }

            if snap.delay_enabled {
                let (dl, dr) = self.delay.process(
                    mix_l,
                    mix_r,
                    snap.delay_time_l,
                    snap.delay_time_r,
                    snap.delay_feedback,
                    snap.delay_mix,
                );
                mix_l = dl;
                mix_r = dr;
            }

            let out_l = mix_l * snap.master_vol;
            let out_r = mix_r * snap.master_vol;

            left[sample_id] = out_l;
            right[sample_id] = out_r;

            self.scope_collector.push(out_l, out_r);
        }
    }
}
