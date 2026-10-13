//! What a voice preset sets on a vocal lane: the singer (bank, speaker,
//! voice type, range) and the delivery. Never the lyrics, the melody or
//! lyric generator settings, or the expression curves — so a singer can
//! be swapped on a finished part.

use serde::{Deserialize, Serialize};

use resonance_music_theory::{VocalParams, VocalTimbre, VoiceType, VoicebankId};

/// The voice and delivery fields of a [`VocalParams`], and nothing else.
///
/// Serialised into a user preset's JSON with `VocalParams`' own field
/// names and enum spellings; a field a file lacks reads as a new lane's
/// default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    pub voicebank: VoicebankId,
    /// The bank's speaker; `None` on a single-voice bank (Lilia).
    pub speaker: Option<String>,
    pub voice: VoiceType,
    /// Melody range in MIDI notes `(low, high)`.
    pub range: (u8, u8),
    pub timbre: VocalTimbre,
    pub tension: f32,
    pub tension_velocity_amount: f32,
    pub tension_contour_amount: f32,
    pub vibrato: f32,
    pub vibrato_rate: f32,
    pub portamento_ms: f32,
    pub articulation: f32,
    pub consonant_emphasis: f32,
    pub velocity_dynamics: f32,
    pub rest_threshold_beats: f32,
    pub bank_timing: bool,
    pub bank_pitch: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self::from_params(&VocalParams::default())
    }
}

/// How close two delivery values must be to count as the same setting
/// (the sliders step by 0.01 at the finest).
const EPSILON: f32 = 1e-4;

impl VoiceSettings {
    /// The voice and delivery `params` carries.
    pub fn from_params(p: &VocalParams) -> Self {
        Self {
            voicebank: p.voicebank.clone(),
            speaker: p.speaker.clone(),
            voice: p.voice,
            range: p.range,
            timbre: p.timbre,
            tension: p.tension,
            tension_velocity_amount: p.tension_velocity_amount,
            tension_contour_amount: p.tension_contour_amount,
            vibrato: p.vibrato,
            vibrato_rate: p.vibrato_rate,
            portamento_ms: p.portamento_ms,
            articulation: p.articulation,
            consonant_emphasis: p.consonant_emphasis,
            velocity_dynamics: p.velocity_dynamics,
            rest_threshold_beats: p.rest_threshold_beats,
            bank_timing: p.bank_timing,
            bank_pitch: p.bank_pitch,
        }
    }

    /// Write these fields onto `p`, leaving the draft, the melody and
    /// lyric generator settings and the expression curves alone.
    pub fn apply_to(&self, p: &mut VocalParams) {
        p.voicebank = self.voicebank.clone();
        p.speaker = self.speaker.clone();
        p.voice = self.voice;
        p.range = self.range;
        p.timbre = self.timbre;
        p.tension = self.tension;
        p.tension_velocity_amount = self.tension_velocity_amount;
        p.tension_contour_amount = self.tension_contour_amount;
        p.vibrato = self.vibrato;
        p.vibrato_rate = self.vibrato_rate;
        p.portamento_ms = self.portamento_ms;
        p.articulation = self.articulation;
        p.consonant_emphasis = self.consonant_emphasis;
        p.velocity_dynamics = self.velocity_dynamics;
        p.rest_threshold_beats = self.rest_threshold_beats;
        p.bank_timing = self.bank_timing;
        p.bank_pitch = self.bank_pitch;
    }

    /// Whether `p` already sings exactly this (delivery values within
    /// [`EPSILON`]). Allocates nothing, so the inspector can ask it of
    /// every preset when it rebuilds.
    pub fn matches(&self, p: &VocalParams) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() <= EPSILON;
        self.voicebank == p.voicebank
            && self.speaker == p.speaker
            && self.voice == p.voice
            && self.range == p.range
            && self.timbre == p.timbre
            && close(self.tension, p.tension)
            && close(self.tension_velocity_amount, p.tension_velocity_amount)
            && close(self.tension_contour_amount, p.tension_contour_amount)
            && close(self.vibrato, p.vibrato)
            && close(self.vibrato_rate, p.vibrato_rate)
            && close(self.portamento_ms, p.portamento_ms)
            && close(self.articulation, p.articulation)
            && close(self.consonant_emphasis, p.consonant_emphasis)
            && close(self.velocity_dynamics, p.velocity_dynamics)
            && close(self.rest_threshold_beats, p.rest_threshold_beats)
            && self.bank_timing == p.bank_timing
            && self.bank_pitch == p.bank_pitch
    }

    /// The bounds [`VocalParams::validate`] holds these fields to, checked
    /// on a default lane carrying them (a user file is hand-editable).
    pub fn validate(&self) -> Result<(), String> {
        let mut p = VocalParams::default();
        self.apply_to(&mut p);
        p.validate().map_err(|e| e.to_string())
    }
}
