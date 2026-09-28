/// Per-voice state for the wavetable synthesizer.
use crate::dsp::analog::{AnalogRng, DriftCoeffs, DriftWalk};
use crate::dsp::envelope::AdsrEnvelope;
use crate::dsp::filter::StateVariableFilter;
use crate::dsp::filter_models::CharacterFilter;
use crate::dsp::lfo::MultiLfo;
use crate::dsp::oscillator::TableTap;
use crate::dsp::sub_noise::{NoiseGen, SubOsc};
use crate::dsp::warp::Warp;

pub const MAX_VOICES: usize = 32;
pub const MAX_UNISON: usize = 7;

#[derive(Clone, Copy, PartialEq)]
pub enum VoiceState {
    Idle,
    Playing,
    Releasing,
}

/// Everything about one unison sub-oscillator that is *not* a function of the
/// oscillator phase, and therefore does not have to be recomputed per sample.
///
/// All of it derives from the block parameter snapshot, the voice's current
/// pitch and the control-rate modulation matrix output. Rebuilding it costs an
/// `exp2` (pitch → Hz), a `log2` (mip-level selection) and a `sin`/`cos` pair
/// (constant-power pan) — roughly 100 cycles. Doing that per sample per unison
/// per oscillator was the synth's dominant cost; the render loop now rebuilds
/// only when one of those inputs actually changes, which for a typical patch
/// is once per control tick rather than once per sample.
#[derive(Clone, Copy, Default)]
pub struct OscSetup {
    /// Phase advance per sample at the resolved frequency.
    pub phase_inc: f64,
    /// Which mip levels / frames to blend, and with what weights.
    pub tap: TableTap,
    /// Oscillator level (osc level × balance), applied before the pan split.
    pub level: f32,
    /// Constant-power pan gains for this sub-voice.
    pub pan_l: f32,
    pub pan_r: f32,
    /// Phase warp resolved at control rate; `Off` unless a warp mode is
    /// selected *and* its amount is non-zero.
    pub warp: Warp,
    /// Height of the step a wrap-jumping warp (Mirror, Formant) puts at the
    /// cycle wrap: the warped value at phase 0 minus the one at phase 1.
    /// Fixed per setup, so the per-sample polyBLEP needs no extra reads.
    pub wrap_jump: f32,
    /// Warped value at phase 0: what a hard-synced slave restarts at.
    pub zero_value: f32,
    /// How far the warped wave moves over the first sample after phase 0:
    /// the slope a restarted slave leaves with, for the sync polyBLAMP.
    pub zero_slope: f32,
}

/// One unison sub-voice: owns its own oscillator phases.
#[derive(Clone)]
pub struct UnisonSubVoice {
    pub osc1_phase: f64,
    pub osc2_phase: f64,
    /// This sub-voice's symmetric position in the unison stack, -1..=1 (0 for
    /// a single sub-voice).
    ///
    /// Detune used to be resolved to absolute cents here at note-on. It is
    /// now the *shape* of the stack only, and the width comes from the block
    /// snapshot at control rate — which is what lets `ModDest::UnisonDetune`
    /// widen or narrow a voice that is already sounding (ba todo #1323).
    pub detune_spread: f32,
    pub pan_offset: f32,
    /// Control-rate cached oscillator setup, refreshed by the render loop.
    pub osc1_setup: OscSetup,
    pub osc2_setup: OscSetup,
    /// polyBLEP residual owed to each oscillator's next sample by a
    /// discontinuity predicted during this one (sync reset, warp wrap jump,
    /// quantize step). Only the interaction/warp kernel writes these; they
    /// are zero on the default path.
    pub osc1_carry: f32,
    pub osc2_carry: f32,
    /// Per-oscillator analog pitch drift, `[-1, 1]`, scaled into cents by
    /// the `analog` knob when the `OscSetup` is rebuilt. The two
    /// oscillators drift independently, as two VCOs would.
    pub osc1_drift: DriftWalk,
    pub osc2_drift: DriftWalk,
}

impl UnisonSubVoice {
    pub fn new() -> Self {
        Self {
            osc1_phase: 0.0,
            osc2_phase: 0.0,
            detune_spread: 0.0,
            pan_offset: 0.0,
            osc1_setup: OscSetup::default(),
            osc2_setup: OscSetup::default(),
            osc1_carry: 0.0,
            osc2_carry: 0.0,
            osc1_drift: DriftWalk::default(),
            osc2_drift: DriftWalk::default(),
        }
    }

    pub fn reset(&mut self) {
        self.osc1_phase = 0.0;
        self.osc2_phase = 0.0;
        self.osc1_carry = 0.0;
        self.osc2_carry = 0.0;
    }
}

/// Per-voice oscillator-interaction amounts, resolved at control rate with
/// the [`OscSetup`]s (the amount is a modulation destination).
#[derive(Clone, Copy, Default)]
pub struct MixSetup {
    /// Phase-modulation depth in cycles per unit of osc2 output.
    pub pm_depth: f64,
    /// Ring-mod wet amount, 0..=1.
    pub ring_wet: f32,
}

/// A single polyphonic voice.
#[derive(Clone)]
pub struct Voice {
    pub state: VoiceState,
    pub note: u8,
    pub velocity: f32,
    pub age: u64,

    // Portamento
    pub current_pitch: f32,
    pub target_pitch: f32,

    // Envelopes
    pub amp_env: AdsrEnvelope,
    pub mod_env: AdsrEnvelope,

    // Per-voice LFO phases (used when retrigger=true)
    pub lfo1: MultiLfo,
    pub lfo2: MultiLfo,
    pub lfo3: MultiLfo,

    // Per-voice stereo filter
    pub filter_l: StateVariableFilter,
    pub filter_r: StateVariableFilter,
    // The same pair for the character models. Only one pair runs at a
    // time — `snap.filter_model` picks — and the engine clears both when
    // the model changes, so neither resumes from stale state.
    pub char_l: CharacterFilter,
    pub char_r: CharacterFilter,
    // Filter FM, refreshed with the coefficients at control rate: the base
    // cutoff as `π·fc/fs` and the FM depth in octaves. Zero depth keeps the
    // per-sample coefficient path switched off.
    pub filter_w: f32,
    pub filter_fm_oct: f32,

    // Unison sub-voices
    pub unison: [UnisonSubVoice; MAX_UNISON],
    pub unison_count: usize,

    // Oscillator interaction, the sub oscillator and the noise source. The
    // sub's increment follows osc1's pitch, so it is refreshed alongside
    // the `OscSetup` caches.
    pub mix_setup: MixSetup,
    pub sub: SubOsc,
    pub sub_inc: f64,
    pub noise: NoiseGen,
    // Analog instability (see `dsp::analog`). The voice's own PRNG, seeded
    // per note-on from the engine's, drives its sub-voices' drift walks.
    // `analog_cutoff` / `analog_level` are this note's static spreads,
    // `[-1, 1]`, scaled by the `analog` knob where they are applied.
    pub analog_rng: AnalogRng,
    pub analog_cutoff: f32,
    pub analog_level: f32,

    // Set by `trigger()`, cleared by the render loop the first time the
    // voice runs through the filter stage. Used to force an immediate
    // filter coefficient update on freshly-triggered voices even when
    // the global control-rate slot wouldn't otherwise tick this sample.
    pub filter_dirty: bool,

    // Same intent as `filter_dirty` but for the modulation matrix
    // snapshot. The mod matrix is evaluated at control rate (every
    // `FILTER_COEFF_INTERVAL` samples) and the result cached in
    // `cached_mods`; this flag forces an immediate re-evaluation on
    // freshly-triggered voices so the first sample uses fresh values.
    pub mod_dirty: bool,

    // Control-rate snapshot of the modulation matrix output. Refreshed
    // every `FILTER_COEFF_INTERVAL` samples (and once on trigger via
    // `mod_dirty`); read by-value per sample inside the render loop.
    pub cached_mods: crate::dsp::modulation::ModState,

    // Drawn at trigger, held for the note's life: backs both
    // `ModSource::RandomBipolar` (read as-is) and `RandomUnipolar` (remapped
    // in `evaluate_mod_matrix`). Not redrawn by `legato()`, for the same
    // reason velocity isn't touched there — changing it under a held note
    // would step the modulation mid-note.
    pub random_value: f32,

    // ±1.0, flipped by the engine on every fresh trigger; backs
    // `ModSource::Alternate`. Also left alone by `legato()`.
    pub alternate_value: f32,

    // Guards the per-unison `OscSetup` caches. The render loop rebuilds them
    // when this is set, or when `current_pitch` has moved away from
    // `osc_setup_pitch` (i.e. portamento is gliding). Set on trigger and
    // whenever the oscillator-relevant modulation outputs change.
    pub osc_setup_dirty: bool,
    pub osc_setup_pitch: f32,

    // "Last computed" values cached per-sample during render. Read by the
    // viz state publisher at the end of each audio block. Not part of the
    // DSP itself.
    pub last_filter_cutoff: f32,
    pub last_osc1_pos: f32,
    pub last_osc2_pos: f32,
    pub last_lfo_phases: [f32; 3],
}

impl Voice {
    pub fn new() -> Self {
        Self {
            state: VoiceState::Idle,
            note: 0,
            velocity: 0.0,
            age: 0,
            current_pitch: 60.0,
            target_pitch: 60.0,
            amp_env: AdsrEnvelope::new(),
            mod_env: AdsrEnvelope::new(),
            lfo1: MultiLfo::new(),
            lfo2: MultiLfo::new(),
            lfo3: MultiLfo::new(),
            filter_l: StateVariableFilter::new(),
            filter_r: StateVariableFilter::new(),
            char_l: CharacterFilter::new(),
            char_r: CharacterFilter::new(),
            filter_w: 0.0,
            filter_fm_oct: 0.0,
            unison: std::array::from_fn(|_| UnisonSubVoice::new()),
            unison_count: 1,
            mix_setup: MixSetup::default(),
            sub: SubOsc::default(),
            sub_inc: 0.0,
            noise: NoiseGen::default(),
            analog_rng: AnalogRng::default(),
            analog_cutoff: 0.0,
            analog_level: 0.0,
            filter_dirty: true,
            mod_dirty: true,
            cached_mods: crate::dsp::modulation::ModState::default(),
            random_value: 0.0,
            alternate_value: 1.0,
            osc_setup_dirty: true,
            osc_setup_pitch: f32::NAN,
            last_filter_cutoff: 8000.0,
            last_osc1_pos: 0.0,
            last_osc2_pos: 0.0,
            last_lfo_phases: [0.0; 3],
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.amp_env.set_sample_rate(sr);
        self.mod_env.set_sample_rate(sr);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn trigger(
        &mut self,
        note: u8,
        velocity: f32,
        age: u64,
        unison_count: usize,
        spread: f32,
        glide: bool,
        lfo1_retrigger: bool,
        lfo2_retrigger: bool,
        lfo3_retrigger: bool,
        random_value: f32,
        alternate_value: f32,
    ) {
        let was_idle = self.state == VoiceState::Idle;
        self.state = VoiceState::Playing;
        self.note = note;
        self.velocity = velocity;
        self.age = age;
        self.target_pitch = note as f32;

        if was_idle || !glide {
            self.current_pitch = note as f32;
        }

        self.amp_env.trigger();
        self.mod_env.trigger();
        self.random_value = random_value;
        self.alternate_value = alternate_value;

        if lfo1_retrigger {
            self.lfo1.reset_phase();
        }
        if lfo2_retrigger {
            self.lfo2.reset_phase();
        }
        if lfo3_retrigger {
            self.lfo3.reset_phase();
        }

        self.clear_filters();
        self.sub = SubOsc::default();
        self.noise = NoiseGen::default();
        self.filter_dirty = true;
        self.mod_dirty = true;
        self.osc_setup_dirty = true;

        // Distribute unison voices
        self.unison_count = unison_count.clamp(1, MAX_UNISON);
        for u in 0..MAX_UNISON {
            self.unison[u].reset();
        }
        distribute_unison(&mut self.unison, self.unison_count, spread);
    }

    /// Zero every filter's state, clean and character alike.
    pub fn clear_filters(&mut self) {
        self.filter_l.clear();
        self.filter_r.clear();
        self.char_l.clear();
        self.char_r.clear();
    }

    /// Draw this note's analog character: reseed the voice PRNG, start every
    /// sub-voice's drift walks, pick the static cutoff / level spreads, and
    /// set the oscillator start phases.
    ///
    /// Called right after [`Self::trigger`] (never after a legato
    /// take-over, whose phases and drift carry on). `phase_random` is the
    /// `osc_phase_random` knob: each start phase is a uniform draw scaled by
    /// it, so 0 leaves `trigger`'s reset-to-zero phases exactly as they were
    /// and 1 is a fully random start — the free-running oscillator of an
    /// analog poly, where a key finds its VCO wherever it happens to be.
    /// The draws are the same whatever the knobs say.
    pub fn seed_analog(&mut self, seed: u32, phase_random: f32, coeffs: &DriftCoeffs) {
        let rng = &mut self.analog_rng;
        *rng = AnalogRng::new(seed);
        self.analog_cutoff = rng.bipolar();
        self.analog_level = rng.bipolar();
        let phase_random = phase_random as f64;
        for sub in self.unison.iter_mut() {
            sub.osc1_drift.start(rng, coeffs);
            sub.osc2_drift.start(rng, coeffs);
            sub.osc1_phase = rng.unit() as f64 * phase_random;
            sub.osc2_phase = rng.unit() as f64 * phase_random;
        }
    }

    /// Take a held voice over for a legato note: move the pitch target
    /// (and, without glide, the pitch) and nothing else. Velocity stays the
    /// first note's, as on a hardware mono synth — changing it here would
    /// step the level mid-note.
    pub fn legato(&mut self, note: u8, age: u64, glide: bool) {
        self.note = note;
        self.age = age;
        self.target_pitch = note as f32;
        if !glide {
            self.current_pitch = note as f32;
        }
        self.mod_dirty = true;
        self.osc_setup_dirty = true;
    }

    pub fn release(&mut self) {
        if self.state == VoiceState::Playing {
            self.state = VoiceState::Releasing;
            self.amp_env.release();
            self.mod_env.release();
        }
    }

    pub fn kill(&mut self) {
        self.state = VoiceState::Idle;
        self.amp_env.reset();
        self.mod_env.reset();
    }
}

/// Distribute unison sub-voices symmetrically across the stereo field and
/// give each its position in the detune spread.
///
/// The detune *width* is deliberately not resolved here — see
/// [`UnisonSubVoice::detune_spread`].
fn distribute_unison(unison: &mut [UnisonSubVoice; MAX_UNISON], count: usize, spread: f32) {
    if count == 1 {
        unison[0].detune_spread = 0.0;
        unison[0].pan_offset = 0.0;
        return;
    }
    for (i, u) in unison.iter_mut().enumerate().take(count) {
        let t = (i as f32 / (count - 1) as f32) * 2.0 - 1.0; // -1 to +1
        u.detune_spread = t;
        u.pan_offset = t * spread;
    }
}

impl Default for UnisonSubVoice {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Voice {
    fn default() -> Self {
        Self::new()
    }
}
