//! Modulation matrix: 8 slots of (source, destination, amount).

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
    /// Display names, indexed by the parameter's integer value. The editor's
    /// source picker reads this array so the labels can never drift from the
    /// discriminants the DSP matches on.
    pub const LABELS: [&'static str; 9] = [
        "None",
        "LFO 1",
        "LFO 2",
        "LFO 3",
        "Mod Env",
        "Velocity",
        "Key Track",
        "Mod Wheel",
        "Aftertouch",
    ];

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

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// `Some(reason)` for a source this build cannot evaluate, so the editor
    /// can offer it as unavailable instead of pretending it works.
    ///
    /// `ModWheel` and `Aftertouch` evaluate to a constant 0.0 because the
    /// framework's `NoteEvent` carries only NoteOn/NoteOff/Choke — no CC,
    /// aftertouch or pitch-bend reaches a plugin at all.
    ///
    /// Implemented by **ba todo #1295** (MIDI CC / aftertouch / pitch-bend
    /// through the CLAP bridge) and then **ba todo #1301** (wire them up as
    /// wavetable mod sources). Delete the arm when #1301 lands.
    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::ModWheel | Self::Aftertouch => {
                Some("No MIDI CC or aftertouch reaches plugins yet (ba todo #1295, #1301)")
            }
            _ => None,
        }
    }

    /// True when this source produces modulation the DSP can actually act on.
    pub fn is_available(self) -> bool {
        self.unavailable_reason().is_none()
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
    /// The master distortion's drive. The master bus is global, not per
    /// voice — see [`ModState::dist_drive`] for how a per-voice matrix
    /// drives it.
    DistDrive = 12,
    /// Per-voice pre-filter saturation (`voice_drive`).
    VoiceDrive = 13,
    FilterFm = 14,
}

impl ModDest {
    /// Display names, indexed by the parameter's integer value. See
    /// [`ModSource::LABELS`] for why these live next to the discriminants.
    pub const LABELS: [&'static str; 15] = [
        "None",
        "Osc1 Position",
        "Osc2 Position",
        "Osc1 Pitch",
        "Osc2 Pitch",
        "Filter Cutoff",
        "Filter Reso",
        "Osc Balance",
        "Amp Level",
        "Unison Detune",
        "Osc1 Pan",
        "Osc2 Pan",
        "Dist Drive",
        "Voice Drive",
        "Filter FM",
    ];

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
            12 => Self::DistDrive,
            13 => Self::VoiceDrive,
            14 => Self::FilterFm,
            _ => Self::None,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// `Some(reason)` for a destination this build accumulates but never
    /// reads, so the editor can offer it as unavailable instead of drawing
    /// a routing that does nothing.
    ///
    /// Every destination is implemented as of ba todo #1323, which wired the
    /// last two (`OscBalance` and `UnisonDetune`) into `refresh_osc_setups`.
    /// The hook stays because it costs nothing and the source side still
    /// needs it — see [`ModSource::unavailable_reason`].
    pub fn unavailable_reason(self) -> Option<&'static str> {
        None
    }

    /// True when modulation sent here changes the sound.
    pub fn is_available(self) -> bool {
        self.unavailable_reason().is_none()
    }
}

/// Summarise what a modulation source is actually routed to, for display
/// next to that source's controls.
///
/// Built from the live matrix rather than a hardcoded string: the LFO cards
/// used to print fixed targets ("→ Wavetable Pos · Cutoff", "→ Macro 4")
/// that described a macro system this synth does not have.
///
/// Slots whose destination is `None` are skipped, as are slots whose amount
/// is zero — a routing that contributes nothing is not a routing the user
/// can hear. A routing to a destination the DSP does not read is listed and
/// marked `(inert)` rather than hidden, so a patch that uses one degrades
/// visibly.
pub fn routing_summary(slots: &[ModSlot], source: ModSource) -> String {
    let mut names: Vec<String> = Vec::new();
    for slot in slots {
        if slot.source != source || slot.dest == ModDest::None || slot.amount == 0.0 {
            continue;
        }
        let name = if slot.dest.is_available() {
            slot.dest.label().to_string()
        } else {
            format!("{} (inert)", slot.dest.label())
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        "not routed".to_string()
    } else {
        format!("→ {}", names.join(" · "))
    }
}

/// Accumulated modulation values for one voice sample.
///
/// `Copy` so the per-sample render kernel can stash a control-rate
/// snapshot on `Voice` and read a cheap by-value copy each sample
/// without holding an active borrow on the voice (which would conflict
/// with mutating `voice.unison[u]` in the inner oscillator loop).
#[derive(Default, Clone, Copy)]
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
    /// Offset to the master distortion's drive, in octaves of drive over
    /// the param's 1..20 range at full scale (see
    /// `render::DIST_DRIVE_MOD_OCTAVES`).
    ///
    /// Every voice accumulates this like any other destination, but the
    /// stage it drives sits on the summed master bus, so only one value can
    /// be used per sample: the render loop takes the **most recently
    /// triggered** sounding voice's (last-note priority, the way a mono
    /// synth resolves the same conflict). For a global source — a free or
    /// synced LFO — every voice holds the same value and the choice is
    /// moot; for a per-voice one (velocity, key track, the mod envelope, a
    /// retriggered LFO) it follows the newest note. With no voice
    /// sounding the last value is held so a release tail does not jump.
    pub dist_drive: f32,
    /// Offset to `voice_drive` (0..1), per voice.
    pub voice_drive: f32,
    /// Offset added to the `filter_fm` amount (0..1 scale).
    pub filter_fm: f32,
}

impl ModState {
    /// True when every field the cached per-unison [`OscSetup`] depends on is
    /// unchanged, so the render loop can keep the cached frequency, mip-level
    /// plan and pan gains instead of recomputing them.
    ///
    /// Deliberately narrower than a full `PartialEq`: the filter and amp
    /// destinations are consumed per sample anyway, and letting a moving
    /// filter LFO invalidate the oscillator cache would defeat the whole
    /// point for the most common patch shape (LFO → cutoff).
    ///
    /// [`OscSetup`]: crate::dsp::voice::OscSetup
    #[inline]
    pub fn osc_setup_eq(&self, other: &Self) -> bool {
        self.osc1_position == other.osc1_position
            && self.osc2_position == other.osc2_position
            && self.osc1_pitch == other.osc1_pitch
            && self.osc2_pitch == other.osc2_pitch
            && self.osc1_pan == other.osc1_pan
            && self.osc2_pan == other.osc2_pan
            // Both feed `OscSetup` (level crossfade, per-unison detune), so a
            // moving one has to invalidate the cache like the others.
            && self.osc_balance == other.osc_balance
            && self.unison_detune == other.unison_detune
    }
}

/// A single modulation routing slot.
pub struct ModSlot {
    pub source: ModSource,
    pub dest: ModDest,
    pub amount: f32,
}

impl ModSlot {
    /// True when this slot is wired at both ends *and* both ends are
    /// implemented, i.e. it can change the sound. The editor uses this for
    /// its "N active" count so the header cannot claim routings that do
    /// nothing.
    pub fn is_effective(&self) -> bool {
        self.source != ModSource::None
            && self.dest != ModDest::None
            && self.source.is_available()
            && self.dest.is_available()
    }
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
        // Unimplemented ends are skipped outright rather than evaluated to
        // a constant 0.0 (sources) or accumulated into a field nothing reads
        // (destinations). Identical output, and there is now exactly one
        // place that decides what this build can modulate — the same place
        // the editor reads to grey the option out. See
        // `ModSource::unavailable_reason` / `ModDest::unavailable_reason`.
        if !slot.is_effective() {
            continue;
        }

        let source_value = match slot.source {
            ModSource::Lfo1 => lfo1_val,
            ModSource::Lfo2 => lfo2_val,
            ModSource::Lfo3 => lfo3_val,
            ModSource::Env2 => mod_env_val * 2.0 - 1.0, // 0..1 -> -1..1
            ModSource::Velocity => velocity * 2.0 - 1.0,
            ModSource::KeyTrack => key_track,
            // Unreachable: filtered by `is_effective` above. Kept exhaustive
            // so #1301 has to come back here when CC delivery lands.
            ModSource::ModWheel | ModSource::Aftertouch | ModSource::None => 0.0,
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
            ModDest::DistDrive => state.dist_drive += mod_value,
            ModDest::VoiceDrive => state.voice_drive += mod_value,
            ModDest::FilterFm => state.filter_fm += mod_value,
            ModDest::None => {}
        }
    }

    state
}
