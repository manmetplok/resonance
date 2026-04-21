/// Per-voice state for the wavetable synthesizer.
use crate::envelope::AdsrEnvelope;
use crate::filter::StateVariableFilter;
use crate::lfo::MultiLfo;

pub const MAX_VOICES: usize = 32;
pub const MAX_UNISON: usize = 7;

#[derive(Clone, Copy, PartialEq)]
pub enum VoiceState {
    Idle,
    Playing,
    Releasing,
}

/// One unison sub-voice: owns its own oscillator phases.
#[derive(Clone)]
pub struct UnisonSubVoice {
    pub osc1_phase: f64,
    pub osc2_phase: f64,
    pub detune_cents: f32,
    pub pan_offset: f32,
}

impl UnisonSubVoice {
    pub fn new() -> Self {
        Self {
            osc1_phase: 0.0,
            osc2_phase: 0.0,
            detune_cents: 0.0,
            pan_offset: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.osc1_phase = 0.0;
        self.osc2_phase = 0.0;
    }
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

    // Unison sub-voices
    pub unison: [UnisonSubVoice; MAX_UNISON],
    pub unison_count: usize,

    // Set by `trigger()`, cleared by the render loop the first time the
    // voice runs through the filter stage. Used to force an immediate
    // filter coefficient update on freshly-triggered voices even when
    // the global control-rate slot wouldn't otherwise tick this sample.
    pub filter_dirty: bool,

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
            unison: std::array::from_fn(|_| UnisonSubVoice::new()),
            unison_count: 1,
            filter_dirty: true,
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

    pub fn trigger(
        &mut self,
        note: u8,
        velocity: f32,
        age: u64,
        unison_count: usize,
        detune_cents: f32,
        spread: f32,
        glide: bool,
        lfo1_retrigger: bool,
        lfo2_retrigger: bool,
        lfo3_retrigger: bool,
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

        if lfo1_retrigger {
            self.lfo1.reset_phase();
        }
        if lfo2_retrigger {
            self.lfo2.reset_phase();
        }
        if lfo3_retrigger {
            self.lfo3.reset_phase();
        }

        self.filter_l.clear();
        self.filter_r.clear();
        self.filter_dirty = true;

        // Distribute unison voices
        self.unison_count = unison_count.clamp(1, MAX_UNISON);
        for u in 0..MAX_UNISON {
            self.unison[u].reset();
        }
        distribute_unison(&mut self.unison, self.unison_count, detune_cents, spread);
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

/// Distribute unison sub-voices symmetrically with detune and stereo spread.
fn distribute_unison(
    unison: &mut [UnisonSubVoice; MAX_UNISON],
    count: usize,
    detune_cents: f32,
    spread: f32,
) {
    if count == 1 {
        unison[0].detune_cents = 0.0;
        unison[0].pan_offset = 0.0;
        return;
    }
    for i in 0..count {
        let t = (i as f32 / (count - 1) as f32) * 2.0 - 1.0; // -1 to +1
        unison[i].detune_cents = t * detune_cents * 0.5;
        unison[i].pan_offset = t * spread;
    }
}
