/// Synth engine: voice allocation, rendering, portamento, effects.
use resonance_dsp::SimpleRng;

use crate::dsp::effects::{Chorus, StereoDelay};
use crate::params::WavetableParams;
use crate::viz::{ScopeCollector, WavetableVizState};
use crate::dsp::voice::{Voice, VoiceState, MAX_VOICES};
use crate::dsp::wavetable::Wavetable;

pub struct SynthEngine {
    pub(crate) voices: Vec<Voice>,
    voice_counter: u64,
    pub(crate) sample_rate: f32,

    // Global LFO phases (used when retrigger=false)
    pub global_lfo1: crate::dsp::lfo::MultiLfo,
    pub global_lfo2: crate::dsp::lfo::MultiLfo,
    pub global_lfo3: crate::dsp::lfo::MultiLfo,

    // Wavetable data
    pub wavetables: Vec<Wavetable>,

    // Effects
    pub(crate) chorus: Chorus,
    pub(crate) delay: StereoDelay,

    // RNG for S&H LFO
    pub(crate) rng: SimpleRng,

    // Last note for portamento
    last_note: Option<u8>,

    // Audio → UI oscilloscope ring. Filled per-sample in `render_block`,
    // published to the shared viz state at the end of each audio block.
    pub(crate) scope_collector: ScopeCollector,
}

impl SynthEngine {
    pub fn new() -> Self {
        Self {
            voices: Vec::new(),
            voice_counter: 0,
            sample_rate: 44100.0,
            global_lfo1: crate::dsp::lfo::MultiLfo::new(),
            global_lfo2: crate::dsp::lfo::MultiLfo::new(),
            global_lfo3: crate::dsp::lfo::MultiLfo::new(),
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
        self.voices = (0..MAX_VOICES)
            .map(|_| {
                let mut v = Voice::new();
                v.set_sample_rate(sample_rate);
                v
            })
            .collect();
        self.voice_counter = 0;
        self.last_note = None;

        // Load pre-generated wavetables from the bundled blob. Generation
        // happens once at plugin build time (see `build.rs`), not on every
        // `initialize()` — this keeps plugin instantiation fast instead of
        // burning multi-seconds on additive synthesis.
        self.wavetables = crate::dsp::wavetable::load_bundled();

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
            viz.store_osc_positions(params.osc1.position.value(), params.osc2.position.value());
            viz.store_lfo_phase(0, self.global_lfo1.phase);
            viz.store_lfo_phase(1, self.global_lfo2.phase);
            viz.store_lfo_phase(2, self.global_lfo3.phase);
        }
    }

    fn find_free_voice(&self, note: u8, max_voices: usize) -> usize {
        // Count active voices
        let active_count = self
            .voices
            .iter()
            .filter(|v| v.state != VoiceState::Idle)
            .count();

        // 1. Prefer idle voice
        if let Some(idx) = self.voices.iter().position(|v| v.state == VoiceState::Idle) {
            if active_count < max_voices {
                return idx;
            }
        }

        // 2. Steal oldest releasing voice
        if let Some((idx, _)) = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.state == VoiceState::Releasing)
            .min_by_key(|(_, v)| v.age)
        {
            return idx;
        }

        // 3. Steal oldest voice with same note
        if let Some((idx, _)) = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.note == note)
            .min_by_key(|(_, v)| v.age)
        {
            return idx;
        }

        // 4. Steal oldest voice overall
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

impl Default for SynthEngine {
    fn default() -> Self {
        Self::new()
    }
}
