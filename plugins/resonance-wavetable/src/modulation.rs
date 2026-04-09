/// Modulation matrix: 8 slots of (source, destination, amount).

pub const NUM_MOD_SLOTS: usize = 8;

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum ModSource {
    None = 0,
    Lfo1 = 1,
    Lfo2 = 2,
    Lfo3 = 3,
    Env2 = 4,
    Velocity = 5,
    KeyTrack = 6,
    ModWheel = 7,
    Aftertouch = 8,
}

impl ModSource {
    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Lfo1,
            2 => Self::Lfo2,
            3 => Self::Lfo3,
            4 => Self::Env2,
            5 => Self::Velocity,
            6 => Self::KeyTrack,
            7 => Self::ModWheel,
            8 => Self::Aftertouch,
            _ => Self::None,
        }
    }

    pub fn name(v: i32) -> &'static str {
        match v {
            0 => "None",
            1 => "LFO 1",
            2 => "LFO 2",
            3 => "LFO 3",
            4 => "Mod Env",
            5 => "Velocity",
            6 => "Key Track",
            7 => "Mod Wheel",
            8 => "Aftertouch",
            _ => "None",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum ModDest {
    None = 0,
    Osc1Position = 1,
    Osc2Position = 2,
    Osc1Pitch = 3,
    Osc2Pitch = 4,
    FilterCutoff = 5,
    FilterResonance = 6,
    OscBalance = 7,
    AmpLevel = 8,
    UnisonDetune = 9,
    Osc1Pan = 10,
    Osc2Pan = 11,
}

impl ModDest {
    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Osc1Position,
            2 => Self::Osc2Position,
            3 => Self::Osc1Pitch,
            4 => Self::Osc2Pitch,
            5 => Self::FilterCutoff,
            6 => Self::FilterResonance,
            7 => Self::OscBalance,
            8 => Self::AmpLevel,
            9 => Self::UnisonDetune,
            10 => Self::Osc1Pan,
            11 => Self::Osc2Pan,
            _ => Self::None,
        }
    }

    pub fn name(v: i32) -> &'static str {
        match v {
            0 => "None",
            1 => "Osc1 Pos",
            2 => "Osc2 Pos",
            3 => "Osc1 Pitch",
            4 => "Osc2 Pitch",
            5 => "Filt Cutoff",
            6 => "Filt Reso",
            7 => "Osc Balance",
            8 => "Amp Level",
            9 => "Uni Detune",
            10 => "Osc1 Pan",
            11 => "Osc2 Pan",
            _ => "None",
        }
    }
}

/// Accumulated modulation values for one voice sample.
#[derive(Default, Clone)]
pub struct ModState {
    pub osc1_position: f32,
    pub osc2_position: f32,
    pub osc1_pitch: f32, // semitones
    pub osc2_pitch: f32,
    pub filter_cutoff: f32, // normalized -1..1 offset
    pub filter_resonance: f32,
    pub osc_balance: f32,
    pub amp_level: f32,
    pub unison_detune: f32,
    pub osc1_pan: f32,
    pub osc2_pan: f32,
}

/// A single modulation routing slot.
pub struct ModSlot {
    pub source: ModSource,
    pub dest: ModDest,
    pub amount: f32,
}

/// Evaluate all modulation slots and return accumulated ModState.
pub fn evaluate_mod_matrix(
    slots: &[ModSlot],
    lfo1_val: f32,
    lfo2_val: f32,
    lfo3_val: f32,
    mod_env_val: f32,
    velocity: f32,
    note: f32,
) -> ModState {
    let mut state = ModState::default();
    let key_track = (note - 60.0) / 60.0; // normalized around middle C

    for slot in slots {
        if slot.source == ModSource::None || slot.dest == ModDest::None {
            continue;
        }

        let source_value = match slot.source {
            ModSource::Lfo1 => lfo1_val,
            ModSource::Lfo2 => lfo2_val,
            ModSource::Lfo3 => lfo3_val,
            ModSource::Env2 => mod_env_val * 2.0 - 1.0, // 0..1 -> -1..1
            ModSource::Velocity => velocity * 2.0 - 1.0,
            ModSource::KeyTrack => key_track,
            ModSource::ModWheel => 0.0, // future: MIDI CC
            ModSource::Aftertouch => 0.0, // future
            ModSource::None => 0.0,
        };

        let mod_value = source_value * slot.amount;

        match slot.dest {
            ModDest::Osc1Position => state.osc1_position += mod_value,
            ModDest::Osc2Position => state.osc2_position += mod_value,
            ModDest::Osc1Pitch => state.osc1_pitch += mod_value * 12.0, // semitones
            ModDest::Osc2Pitch => state.osc2_pitch += mod_value * 12.0,
            ModDest::FilterCutoff => state.filter_cutoff += mod_value,
            ModDest::FilterResonance => state.filter_resonance += mod_value,
            ModDest::OscBalance => state.osc_balance += mod_value,
            ModDest::AmpLevel => state.amp_level += mod_value,
            ModDest::UnisonDetune => state.unison_detune += mod_value,
            ModDest::Osc1Pan => state.osc1_pan += mod_value,
            ModDest::Osc2Pan => state.osc2_pan += mod_value,
            ModDest::None => {}
        }
    }

    state
}
