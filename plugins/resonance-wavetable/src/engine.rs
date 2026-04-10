/// Synth engine: voice allocation, rendering, portamento, effects.

use resonance_dsp::{constant_power_pan, SimpleRng};

use crate::effects::{Chorus, Distortion, StereoDelay};
use crate::filter::FilterType;
use crate::lfo::LfoShape;
use crate::modulation::{self, ModDest, ModSlot, ModSource, NUM_MOD_SLOTS};
use crate::oscillator::{self, midi_to_freq, read_wavetable};
use crate::params::WavetableParams;
use crate::viz::{ScopeCollector, WavetableVizState};
use crate::voice::{Voice, VoiceState, MAX_VOICES};
use crate::wavetable::Wavetable;

pub struct SynthEngine {
    voices: Vec<Voice>,
    voice_counter: u64,
    sample_rate: f32,

    // Global LFO phases (used when retrigger=false)
    pub global_lfo1: crate::lfo::MultiLfo,
    pub global_lfo2: crate::lfo::MultiLfo,
    pub global_lfo3: crate::lfo::MultiLfo,

    // Wavetable data
    pub wavetables: Vec<Wavetable>,

    // Effects
    chorus: Chorus,
    delay: StereoDelay,

    // RNG for S&H LFO
    rng: SimpleRng,

    // Last note for portamento
    last_note: Option<u8>,

    // Audio → UI oscilloscope ring. Filled per-frame in render_frame,
    // published to the shared viz state at the end of each audio block.
    scope_collector: ScopeCollector,
}

impl SynthEngine {
    pub fn new() -> Self {
        Self {
            voices: Vec::new(),
            voice_counter: 0,
            sample_rate: 44100.0,
            global_lfo1: crate::lfo::MultiLfo::new(),
            global_lfo2: crate::lfo::MultiLfo::new(),
            global_lfo3: crate::lfo::MultiLfo::new(),
            wavetables: Vec::new(),
            chorus: Chorus::new(44100.0),
            delay: StereoDelay::new(44100.0),
            rng: SimpleRng::new(42),
            last_note: None,
            scope_collector: ScopeCollector::new(),
        }
    }

    pub fn initialize(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.voices = (0..MAX_VOICES).map(|_| {
            let mut v = Voice::new();
            v.set_sample_rate(sample_rate);
            v
        }).collect();
        self.voice_counter = 0;
        self.last_note = None;

        // Generate wavetables
        self.wavetables = crate::wavetable::generate_all(sample_rate);

        // Init effects
        self.chorus = Chorus::new(sample_rate);
        self.delay = StereoDelay::new(sample_rate);
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.kill();
        }
        self.voice_counter = 0;
        self.last_note = None;
        self.global_lfo1.reset_phase();
        self.global_lfo2.reset_phase();
        self.global_lfo3.reset_phase();
        self.chorus.reset();
        self.delay.reset();
    }

    pub fn note_on(&mut self, note: u8, velocity: f32, params: &WavetableParams) {
        let max_v = params.max_voices.value().max(1) as usize;
        let voice_idx = self.find_free_voice(note, max_v);

        self.voice_counter += 1;
        let glide = params.glide_enabled.value() && self.last_note.is_some();
        let unison_count = params.unison.voices.value().max(1) as usize;
        let detune = params.unison.detune.value();
        let spread = params.unison.spread.value();

        let voice = &mut self.voices[voice_idx];
        voice.trigger(
            note,
            velocity,
            self.voice_counter,
            unison_count,
            detune,
            spread,
            glide,
            params.lfo1.retrigger.value(),
            params.lfo2.retrigger.value(),
            params.lfo3.retrigger.value(),
        );

        self.last_note = Some(note);
    }

    pub fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.state == VoiceState::Playing && voice.note == note {
                voice.release();
            }
        }
    }

    /// Render one stereo frame.
    pub fn render_frame(&mut self, params: &WavetableParams) -> (f32, f32) {
        // Cache global params
        let master_vol = params.master_volume.value();
        let osc_balance = params.osc_balance.value();
        let osc1_enabled = params.osc1.enabled.value();
        let osc2_enabled = params.osc2.enabled.value();
        let osc1_wt = params.osc1.wavetable.value() as usize;
        let osc2_wt = params.osc2.wavetable.value() as usize;
        let osc1_pos = params.osc1.position.value();
        let osc2_pos = params.osc2.position.value();
        let osc1_coarse = params.osc1.coarse.value() as f32;
        let osc2_coarse = params.osc2.coarse.value() as f32;
        let osc1_fine = params.osc1.fine.value();
        let osc2_fine = params.osc2.fine.value();
        let osc1_level = params.osc1.level.value();
        let osc2_level = params.osc2.level.value();
        let osc1_pan = params.osc1.pan.value();
        let osc2_pan = params.osc2.pan.value();

        let filter_enabled = params.filter.enabled.value();
        let filter_type = FilterType::from_int(params.filter.filter_type.value());
        let filter_cutoff = params.filter.cutoff.value();
        let filter_reso = params.filter.resonance.value();
        let filter_env_depth = params.filter.env_depth.value();
        let filter_keytrack = params.filter.keytrack.value();
        let filter_drive = params.filter.drive.value();

        let amp_attack = params.amp_env.attack.value();
        let amp_decay = params.amp_env.decay.value();
        let amp_sustain = params.amp_env.sustain.value();
        let amp_release = params.amp_env.release.value();
        let amp_curve = params.amp_env.curve.value();

        let mod_attack = params.mod_env.attack.value();
        let mod_decay = params.mod_env.decay.value();
        let mod_sustain = params.mod_env.sustain.value();
        let mod_release = params.mod_env.release.value();
        let mod_curve = params.mod_env.curve.value();

        let lfo1_shape = LfoShape::from_int(params.lfo1.shape.value());
        let lfo1_rate = params.lfo1.rate.value();
        let lfo1_depth = params.lfo1.depth.value();
        let lfo1_retrigger = params.lfo1.retrigger.value();

        let lfo2_shape = LfoShape::from_int(params.lfo2.shape.value());
        let lfo2_rate = params.lfo2.rate.value();
        let lfo2_depth = params.lfo2.depth.value();
        let lfo2_retrigger = params.lfo2.retrigger.value();

        let lfo3_shape = LfoShape::from_int(params.lfo3.shape.value());
        let lfo3_rate = params.lfo3.rate.value();
        let lfo3_depth = params.lfo3.depth.value();
        let lfo3_retrigger = params.lfo3.retrigger.value();

        let glide_enabled = params.glide_enabled.value();
        let glide_time_ms = params.glide_time.value();
        let glide_coeff = if glide_enabled && glide_time_ms > 0.0 {
            1.0 - (-1.0 / (glide_time_ms * 0.001 * self.sample_rate)).exp()
        } else {
            1.0 // instant
        };

        // Build mod slots
        let mod_slots: [ModSlot; NUM_MOD_SLOTS] = std::array::from_fn(|i| ModSlot {
            source: ModSource::from_int(params.mod_slots[i].source.value()),
            dest: ModDest::from_int(params.mod_slots[i].destination.value()),
            amount: params.mod_slots[i].amount.value(),
        });

        // Advance global LFOs
        self.global_lfo1.set_rate(lfo1_rate, self.sample_rate);
        self.global_lfo2.set_rate(lfo2_rate, self.sample_rate);
        self.global_lfo3.set_rate(lfo3_rate, self.sample_rate);
        let global_lfo1_val = self.global_lfo1.next(lfo1_shape, &mut self.rng) * lfo1_depth;
        let global_lfo2_val = self.global_lfo2.next(lfo2_shape, &mut self.rng) * lfo2_depth;
        let global_lfo3_val = self.global_lfo3.next(lfo3_shape, &mut self.rng) * lfo3_depth;

        let mut mix_l = 0.0f32;
        let mut mix_r = 0.0f32;

        let wt1 = if osc1_wt < self.wavetables.len() { Some(&self.wavetables[osc1_wt]) } else { None };
        let wt2 = if osc2_wt < self.wavetables.len() { Some(&self.wavetables[osc2_wt]) } else { None };

        for voice in &mut self.voices {
            if voice.state == VoiceState::Idle {
                continue;
            }

            // Portamento
            voice.current_pitch += (voice.target_pitch - voice.current_pitch) * glide_coeff;

            // LFO values (per-voice or global)
            voice.lfo1.set_rate(lfo1_rate, self.sample_rate);
            voice.lfo2.set_rate(lfo2_rate, self.sample_rate);
            voice.lfo3.set_rate(lfo3_rate, self.sample_rate);

            let lfo1_val = if lfo1_retrigger {
                voice.lfo1.next(lfo1_shape, &mut self.rng) * lfo1_depth
            } else {
                global_lfo1_val
            };
            let lfo2_val = if lfo2_retrigger {
                voice.lfo2.next(lfo2_shape, &mut self.rng) * lfo2_depth
            } else {
                global_lfo2_val
            };
            let lfo3_val = if lfo3_retrigger {
                voice.lfo3.next(lfo3_shape, &mut self.rng) * lfo3_depth
            } else {
                global_lfo3_val
            };

            // Envelopes
            let amp_env_val = voice.amp_env.next(
                amp_attack, amp_decay, amp_sustain, amp_release, amp_curve,
            );
            let mod_env_val = voice.mod_env.next(
                mod_attack, mod_decay, mod_sustain, mod_release, mod_curve,
            );

            // Check if voice has finished
            if voice.amp_env.is_idle() && voice.state == VoiceState::Releasing {
                voice.state = VoiceState::Idle;
                continue;
            }

            // Modulation matrix
            let mods = modulation::evaluate_mod_matrix(
                &mod_slots,
                lfo1_val,
                lfo2_val,
                lfo3_val,
                mod_env_val,
                voice.velocity,
                voice.current_pitch,
            );

            // Render oscillators with unison
            let mut osc_l = 0.0f32;
            let mut osc_r = 0.0f32;

            for u in 0..voice.unison_count {
                let sub = &mut voice.unison[u];

                if osc1_enabled {
                    if let Some(wt) = wt1 {
                        let pitch = voice.current_pitch
                            + osc1_coarse
                            + osc1_fine / 100.0
                            + sub.detune_cents / 100.0
                            + mods.osc1_pitch;
                        let freq = midi_to_freq(pitch);
                        let pos = (osc1_pos + mods.osc1_position).clamp(0.0, 1.0);
                        let sample = read_wavetable(wt, sub.osc1_phase, pos, freq);
                        sub.osc1_phase += oscillator::phase_inc(freq, self.sample_rate);
                        sub.osc1_phase -= sub.osc1_phase.floor();

                        let pan = (osc1_pan + sub.pan_offset + mods.osc1_pan).clamp(-1.0, 1.0);
                        let (pl, pr) = constant_power_pan(pan);
                        let level = osc1_level * (1.0 - osc_balance.max(0.0)); // balance attenuates
                        osc_l += sample * level * pl;
                        osc_r += sample * level * pr;
                    }
                }

                if osc2_enabled {
                    if let Some(wt) = wt2 {
                        let pitch = voice.current_pitch
                            + osc2_coarse
                            + osc2_fine / 100.0
                            + sub.detune_cents / 100.0
                            + mods.osc2_pitch;
                        let freq = midi_to_freq(pitch);
                        let pos = (osc2_pos + mods.osc2_position).clamp(0.0, 1.0);
                        let sample = read_wavetable(wt, sub.osc2_phase, pos, freq);
                        sub.osc2_phase += oscillator::phase_inc(freq, self.sample_rate);
                        sub.osc2_phase -= sub.osc2_phase.floor();

                        let pan = (osc2_pan + sub.pan_offset + mods.osc2_pan).clamp(-1.0, 1.0);
                        let (pl, pr) = constant_power_pan(pan);
                        let level = osc2_level * (1.0 + osc_balance.min(0.0).abs()); // balance boosts
                        osc_l += sample * level * pl;
                        osc_r += sample * level * pr;
                    }
                }
            }

            // Normalize unison
            let unison_scale = 1.0 / (voice.unison_count as f32).sqrt();
            osc_l *= unison_scale;
            osc_r *= unison_scale;

            // Filter
            if filter_enabled {
                let key_offset = filter_keytrack * (voice.current_pitch - 60.0) / 12.0;
                let env_offset = filter_env_depth * mod_env_val;
                // Cutoff in Hz with key tracking and env mod
                let cutoff = filter_cutoff
                    * 2.0f32.powf(key_offset + env_offset * 5.0 + mods.filter_cutoff * 5.0);
                let cutoff = cutoff.clamp(20.0, 20000.0);
                let reso = (filter_reso + mods.filter_resonance).clamp(0.0, 1.0);

                osc_l = voice.filter_l.process(
                    osc_l, cutoff, reso, self.sample_rate, filter_type, filter_drive,
                );
                osc_r = voice.filter_r.process(
                    osc_r, cutoff, reso, self.sample_rate, filter_type, filter_drive,
                );
                voice.last_filter_cutoff = cutoff;
            } else {
                voice.last_filter_cutoff = filter_cutoff;
            }

            // Cache post-modulation osc positions + LFO phases for viz.
            voice.last_osc1_pos = (osc1_pos + mods.osc1_position).clamp(0.0, 1.0);
            voice.last_osc2_pos = (osc2_pos + mods.osc2_position).clamp(0.0, 1.0);
            voice.last_lfo_phases[0] = if lfo1_retrigger { voice.lfo1.phase } else { self.global_lfo1.phase };
            voice.last_lfo_phases[1] = if lfo2_retrigger { voice.lfo2.phase } else { self.global_lfo2.phase };
            voice.last_lfo_phases[2] = if lfo3_retrigger { voice.lfo3.phase } else { self.global_lfo3.phase };

            // Amplitude
            let amp = amp_env_val * voice.velocity * (1.0 + mods.amp_level).max(0.0);
            mix_l += osc_l * amp;
            mix_r += osc_r * amp;
        }

        // Effects chain
        let dist_enabled = params.distortion.enabled.value();
        let chorus_enabled = params.chorus.enabled.value();
        let delay_enabled = params.delay.enabled.value();

        if dist_enabled {
            let drive = params.distortion.drive.value();
            let dmix = params.distortion.mix.value();
            let (dl, dr) = Distortion::process(mix_l, mix_r, drive, dmix);
            mix_l = dl;
            mix_r = dr;
        }

        if chorus_enabled {
            let rate = params.chorus.rate.value();
            let depth = params.chorus.depth.value();
            let cmix = params.chorus.mix.value();
            let (cl, cr) = self.chorus.process(mix_l, mix_r, rate, depth, cmix);
            mix_l = cl;
            mix_r = cr;
        }

        if delay_enabled {
            let tl = params.delay.time_l.value();
            let tr = params.delay.time_r.value();
            let fb = params.delay.feedback.value();
            let dmix = params.delay.mix.value();
            let (dl, dr) = self.delay.process(mix_l, mix_r, tl, tr, fb, dmix);
            mix_l = dl;
            mix_r = dr;
        }

        let out_l = mix_l * master_vol;
        let out_r = mix_r * master_vol;

        // Feed the oscilloscope ring. The publish to shared atomics happens
        // once per block via publish_viz below.
        self.scope_collector.push(out_l, out_r);

        (out_l, out_r)
    }

    /// Publish the latest audio-thread state to the shared viz atomics.
    /// Called once per audio block by the plugin's `process()`.
    pub fn publish_viz(&mut self, params: &WavetableParams, viz: &WavetableVizState) {
        // Flush the oscilloscope buffer.
        self.scope_collector.publish(viz);

        // Pick the representative voice: the newest non-idle one. If nothing
        // is active we leave the scalars at their previous values, which
        // avoids glitchy snap-to-zero between notes.
        let mut rep_idx: Option<usize> = None;
        let mut rep_age: u64 = 0;
        let mut active = 0u32;
        for (i, v) in self.voices.iter().enumerate() {
            if v.state != VoiceState::Idle {
                active += 1;
                if v.age >= rep_age {
                    rep_age = v.age;
                    rep_idx = Some(i);
                }
            }
        }
        viz.store_active_voice_count(active);

        if let Some(i) = rep_idx {
            let voice = &self.voices[i];
            viz.store_env_amp(voice.amp_env.level, voice.amp_env.stage as u32);
            viz.store_env_mod(voice.mod_env.level);
            viz.store_filter_cutoff_live(voice.last_filter_cutoff);
            viz.store_osc_positions(voice.last_osc1_pos, voice.last_osc2_pos);
            for (lfo, phase) in voice.last_lfo_phases.iter().enumerate() {
                viz.store_lfo_phase(lfo, *phase);
            }
        } else {
            // No active voices: reflect the current params where it makes
            // sense so the UI still shows sensible values when idle.
            viz.store_filter_cutoff_live(params.filter.cutoff.value());
            viz.store_osc_positions(
                params.osc1.position.value(),
                params.osc2.position.value(),
            );
            viz.store_lfo_phase(0, self.global_lfo1.phase);
            viz.store_lfo_phase(1, self.global_lfo2.phase);
            viz.store_lfo_phase(2, self.global_lfo3.phase);
        }
    }

    fn find_free_voice(&self, note: u8, max_voices: usize) -> usize {
        // Count active voices
        let active_count = self.voices.iter().filter(|v| v.state != VoiceState::Idle).count();

        // 1. Prefer idle voice
        if let Some(idx) = self.voices.iter().position(|v| v.state == VoiceState::Idle) {
            if active_count < max_voices {
                return idx;
            }
        }

        // 2. Steal oldest releasing voice
        if let Some((idx, _)) = self.voices.iter().enumerate()
            .filter(|(_, v)| v.state == VoiceState::Releasing)
            .min_by_key(|(_, v)| v.age)
        {
            return idx;
        }

        // 3. Steal oldest voice with same note
        if let Some((idx, _)) = self.voices.iter().enumerate()
            .filter(|(_, v)| v.note == note)
            .min_by_key(|(_, v)| v.age)
        {
            return idx;
        }

        // 4. Steal oldest voice overall
        self.voices.iter().enumerate()
            .min_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}
