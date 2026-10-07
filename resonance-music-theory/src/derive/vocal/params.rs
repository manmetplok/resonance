//! Vocal-generator parameter enums + `VocalParams` struct + all the
//! `#[serde(default)]` helper fns. Pure data; no rng, no melody logic.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::lyrics::LyricLine;

mod legacy;

/// Lyrical mood preset. Drives the lyric generator's word choice and
/// chord-mood pairing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum VocalMood {
    Yearning,
    Defiant,
    Hopeful,
    Reflective,
    Joyful,
    Melancholy,
}

impl VocalMood {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Lyrical point of view.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum VocalPov {
    #[strum(serialize = "1st singular")]
    FirstSingular,
    #[strum(serialize = "1st plural")]
    FirstPlural,
    #[strum(serialize = "2nd person")]
    SecondPerson,
    #[strum(serialize = "3rd person")]
    ThirdPerson,
    Narrator,
}

impl VocalPov {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// End-rhyme scheme applied to the four (or N) generated lines.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum VocalRhymeScheme {
    #[strum(serialize = "AABB")]
    Aabb,
    #[strum(serialize = "ABAB")]
    Abab,
    #[strum(serialize = "ABCB")]
    Abcb,
    #[strum(serialize = "ABBA")]
    Abba,
    Free,
}

impl VocalRhymeScheme {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Voice type / tessitura preset.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum VoiceType {
    Soprano,
    #[strum(serialize = "Mezzo")]
    MezzoSoprano,
    Alto,
    Tenor,
    Baritone,
    Bass,
}

impl VoiceType {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Default low/high MIDI note range for the voice type.
    pub fn default_range(self) -> (u8, u8) {
        match self {
            VoiceType::Soprano => (60, 84),       // C4..C6
            VoiceType::MezzoSoprano => (57, 79),  // A3..G5
            VoiceType::Alto => (55, 77),          // G3..F5
            VoiceType::Tenor => (48, 72),         // C3..C5
            VoiceType::Baritone => (43, 67),      // G2..G4
            VoiceType::Bass => (40, 64),          // E2..E4
        }
    }
}

/// Note → syllable mapping mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum SyllableMode {
    /// One note per syllable.
    Syllabic,
    /// Mostly syllabic, with occasional held notes across two syllables.
    Mixed,
    /// Multi-note holds — one syllable stretched over several notes.
    Melismatic,
}

impl SyllableMode {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Vocal phrase-contour family. Mirrors `ContourPreference` but kept
/// separate so the vocal rail can present its own glyph set (arch, rise,
/// fall, wave, flat) without polluting the instrument-melody enum.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum VocalContour {
    Arch,
    Rise,
    Fall,
    Wave,
    Flat,
}

impl VocalContour {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Melodic style preset. Drives both pitch selection and the rhythmic
/// feel — different styles produce genuinely different singers, not just
/// the same algorithm with shifted sliders.
///
/// Each variant maps to its own per-syllable generator inside
/// `derive_vocal`. The other "Melody" sliders (contour, anchor, leap)
/// still apply, but the style governs the dominant character — e.g.
/// `Hymnal` always walks in steps regardless of the leap slider, and
/// `Chant` ignores the contour because it sits on one pitch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum VocalStyle {
    /// Stepwise, breath-driven, contour-shaped — Sting / Adele / Sade.
    /// The legacy default, kept for backwards compatibility.
    #[strum(serialize = "Pop ballad")]
    PopBallad,
    /// Talky / spoken: pitches cluster around a "speaking note", lots of
    /// repeated pitches, small inflections at line edges.
    Conversational,
    /// Hymn / lullaby: strict syllable-per-quarter, stepwise only,
    /// narrow range, every line cadences to a chord tone.
    Hymnal,
    /// Pentatonic, descending phrases, long–short rhythm pairs, lines
    /// echo their own contour two lines later.
    Folk,
    /// Wide range with a peak roughly mid-line, sustained final note,
    /// leaps to chord-tone climaxes — power-ballad chorus energy.
    Anthemic,
    /// Hip-hop / spoken-word: monotone-leaning with bursts of fast
    /// syllables and short rests between them.
    Chant,
}

impl VocalStyle {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// A voicebank's string id — `tiger`, `lilia`, `meiji`, or the folder id
/// of any bank installed in the user's voicebank directory
/// (vocal-mcp-control-and-quality.md §4.1). Serialised as a plain string.
/// Ids are lowercase; [`VoicebankId::new`] trims and lowercases, so the
/// old enum names a project file may still carry (`Tiger`, `Lilia`,
/// `Meiji`) read back as the ids they became.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct VoicebankId(String);

impl VoicebankId {
    /// Lilia, the default bank (spec D9).
    pub const DEFAULT: &'static str = "lilia";

    /// An id from any spelling of it: trimmed and lowercased.
    pub fn new(id: impl AsRef<str>) -> Self {
        Self(id.as_ref().trim().to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for VoicebankId {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl std::fmt::Display for VoicebankId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for VoicebankId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Self::new)
    }
}

impl PartialEq<str> for VoicebankId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for VoicebankId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Vocal-line timbre preset.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString, strum::VariantArray,
)]
pub enum VocalTimbre {
    Airy,
    Warm,
    Edged,
    Bright,
}

impl VocalTimbre {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// All vocal-generator parameters surfaced in the right rail. Persists
/// alongside the rest of the lane generator configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "legacy::VocalParamsRepr")]
pub struct VocalParams {
    // ---- Lyrics ----
    pub theme: String,
    pub mood: VocalMood,
    pub pov: VocalPov,
    pub rhyme: VocalRhymeScheme,
    pub lines: u8,
    pub syllables_min: u8,
    pub syllables_max: u8,
    pub match_syllables_to_melody: bool,
    pub avoid_cliches: bool,

    // ---- Lyric draft preview (in-state, regenerated on demand) ----
    pub draft: Vec<LyricLine>,

    // ---- Melody ----
    pub voice: VoiceType,
    pub range: (u8, u8),
    /// Melodic style preset. Picks the per-syllable generator used by
    /// `derive_vocal`. Defaults to `PopBallad` (the legacy behavior) so
    /// older project files round-trip unchanged.
    pub style: VocalStyle,
    pub contour: VocalContour,
    pub syllable_mode: SyllableMode,
    /// Probability a strong-beat syllable lands on a chord tone (root /
    /// 3rd / 5th). Range 0.0..=1.0.
    pub chord_tone_anchor: f32,
    /// Leap-vs-step bias. 0.0 = always step, 1.0 = always leap.
    pub leap_range: f32,
    /// Length of a melodic phrase in bars before a breath/rest.
    pub phrase_length_bars: u8,
    /// Breath gap between phrases (0.0..=1.0).
    pub breath: f32,
    pub stay_in_scale: bool,
    pub avoid_clashes: bool,
    /// Reuse the section's shared motif (intervals only) for the vocal
    /// melody. The chosen `style`'s rhythm, dynamics, breath gaps and
    /// cadence handling still drive the surface — only the per-syllable
    /// pitches are replaced with the motif's signed intervals (added to
    /// the chord root in range, snapped to scale). The final syllable
    /// of each line keeps the style's `cadence_pitch` landing so phrases
    /// still resolve, even with a non-resolving motif.
    pub use_section_motif: bool,

    // ---- Voice & delivery ----
    pub timbre: VocalTimbre,
    pub vibrato: f32,
    /// Vibrato rate in Hz. Real singers run between roughly 4 Hz
    /// (slow / classical) and 7 Hz (operatic / pop pressure). Defaults
    /// to 5 Hz, the historic hardcoded value.
    pub vibrato_rate: f32,
    /// Baseline voice tension in [-1.0, +1.0]. -1 = relaxed /
    /// breathy delivery, 0 = neutral, +1 = compressed / belted.
    /// Mapped to the SVS model's `tension` per-frame input on
    /// voicebanks that accept it (Lilia, Meiji); ignored for
    /// voicebanks that don't (TIGER). Per-frame modulators (see
    /// `tension_velocity_amount`, `tension_contour_amount`) add to
    /// this baseline so the actual tension can vary throughout the
    /// section.
    pub tension: f32,
    /// How much the per-syllable note velocity modulates tension
    /// dynamically. 0 = constant tension equal to the slider; 1 =
    /// strong syllables (accented beats) push tension up by ~+0.5,
    /// weak syllables push it down by ~-0.3 — re-creating the way
    /// real singers tense up on accents.
    pub tension_velocity_amount: f32,
    /// How much the section's pitch contour modulates tension
    /// dynamically. 0 = no contour modulation; 1 = the highest notes
    /// of the section push tension up by ~+0.5 (singers belt at the
    /// top of their range), the lowest notes push it down by ~-0.5
    /// (more relaxed delivery on low notes).
    pub tension_contour_amount: f32,
    /// Pitch portamento (note-to-note glide) duration in milliseconds.
    /// Range ~10–200 ms. Lower = harder note transitions (tighter,
    /// more "snap"); higher = smoother glides (legato, scoopy). The
    /// SVS pipeline applies this as a linear ramp inserted before each
    /// pitch change. Defaults to 40 ms — the historic hardcoded value
    /// that matches the reference singing fixtures.
    pub portamento_ms: f32,
    pub articulation: f32,
    pub consonant_emphasis: f32,
    /// The voicebank's speaker (voice mode), as the bank's speaker id
    /// (`disco` on TIGER, `phantom` on Meiji, ...); `None` sings with the
    /// bank's default speaker, and is all a single-voice bank (Lilia)
    /// ever holds. Replaces the old per-bank `singer` (TIGER) and
    /// `singer_meiji` (Meiji) fields, which loading maps onto it
    /// (`legacy`).
    pub speaker: Option<String>,
    /// Trained DiffSinger model used for the SVS render, by id. Defaults
    /// to `lilia`, also for older JSON without the field.
    pub voicebank: VoicebankId,
}

impl Default for VocalParams {
    fn default() -> Self {
        let voice = VoiceType::Alto;
        let range = voice.default_range();
        Self {
            theme: String::from(
                "A house made of glass — fragile loves, the stones we can't take back.",
            ),
            mood: VocalMood::Yearning,
            pov: VocalPov::FirstSingular,
            rhyme: VocalRhymeScheme::Abab,
            lines: 4,
            syllables_min: 8,
            syllables_max: 11,
            match_syllables_to_melody: true,
            avoid_cliches: true,
            draft: default_draft(),
            voice,
            range,
            style: default_vocal_style(),
            contour: VocalContour::Arch,
            syllable_mode: SyllableMode::Syllabic,
            chord_tone_anchor: 0.65,
            // Pop/folk singing is mostly stepwise — 15% leap probability
            // gives a singable melodic line. The old 30% default plus
            // wide chord-tone snapping produced octave-jumping vocal
            // melodies that the SVS model rendered as glitched audio.
            leap_range: 0.15,
            phrase_length_bars: 2,
            breath: 0.45,
            stay_in_scale: true,
            avoid_clashes: true,
            use_section_motif: false,
            timbre: VocalTimbre::Warm,
            vibrato: 0.30,
            vibrato_rate: default_vibrato_rate(),
            tension: default_tension(),
            tension_velocity_amount: default_tension_velocity_amount(),
            tension_contour_amount: default_tension_contour_amount(),
            portamento_ms: default_portamento_ms(),
            articulation: 0.65,
            // Consonant emphasis lives in a [0.40, 0.60] "perfect
            // intelligibility" band per the whisper sweep on Lilia
            // — below 0.40 (= ~55 ms consonants) the lyric collapses
            // to garbage, above 0.60 the model starts mis-hearing
            // similar-sounding endings. Default to the middle for a
            // safety margin from either cliff.
            consonant_emphasis: 0.50,
            speaker: None,
            voicebank: VoicebankId::default(),
        }
    }
}

/// Things that can be wrong with a [`VocalParams`] at the app boundary.
/// Each variant points at the offending field so the UI can surface a
/// targeted error.
#[derive(Debug, Error, PartialEq)]
pub enum VocalParamsError {
    #[error("lines must be >= 1 (got {0})")]
    LinesTooLow(u8),
    #[error("syllables_min ({min}) must be <= syllables_max ({max}) and >= 1")]
    SyllablesRange { min: u8, max: u8 },
    #[error("range {lo}..{hi} must satisfy lo < hi and both within MIDI 0..=127")]
    BadRange { lo: u8, hi: u8 },
    #[error("phrase_length_bars must be >= 1 (got {0})")]
    PhraseLengthZero(u8),
    #[error("{field} must be within [{lo}, {hi}] (got {value})")]
    OutOfRange {
        field: &'static str,
        value: f32,
        lo: f32,
        hi: f32,
    },
}

impl VocalParams {
    /// Longest lyric theme, in characters. The lane inspector truncates
    /// to it; the control API refuses longer text.
    pub const THEME_MAX_CHARS: usize = 240;

    /// Switch to `voicebank`, resetting the speaker to `default_speaker`
    /// (the new bank's default, which the caller looks up: the app's
    /// voicebank registry knows each bank's speakers) when the bank
    /// actually changes. Re-selecting the current bank changes nothing.
    /// The one rule every bank switch goes through (`vocal.set_params`,
    /// `vocal.render {voicebank}`, the lane inspector's picker).
    pub fn switch_voicebank(&mut self, voicebank: VoicebankId, default_speaker: Option<String>) {
        if voicebank != self.voicebank {
            self.speaker = default_speaker;
        }
        self.voicebank = voicebank;
    }

    /// Bounds-check the implicit invariants on each field. Run at the
    /// app boundary before handing params to the engine so a malformed
    /// project file or a UI state-machine bug surfaces as a clean
    /// error rather than a panic deep in `derive_vocal` or the SVS
    /// pipeline.
    pub fn validate(&self) -> Result<(), VocalParamsError> {
        if self.lines < 1 {
            return Err(VocalParamsError::LinesTooLow(self.lines));
        }
        if self.syllables_min < 1 || self.syllables_min > self.syllables_max {
            return Err(VocalParamsError::SyllablesRange {
                min: self.syllables_min,
                max: self.syllables_max,
            });
        }
        let (lo, hi) = self.range;
        if lo >= hi || hi > 127 {
            return Err(VocalParamsError::BadRange { lo, hi });
        }
        if self.phrase_length_bars < 1 {
            return Err(VocalParamsError::PhraseLengthZero(self.phrase_length_bars));
        }

        // f32 [0, 1] sliders.
        for (field, value) in [
            ("chord_tone_anchor", self.chord_tone_anchor),
            ("leap_range", self.leap_range),
            ("breath", self.breath),
            ("vibrato", self.vibrato),
            ("articulation", self.articulation),
            ("consonant_emphasis", self.consonant_emphasis),
            ("tension_velocity_amount", self.tension_velocity_amount),
            ("tension_contour_amount", self.tension_contour_amount),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(VocalParamsError::OutOfRange {
                    field,
                    value,
                    lo: 0.0,
                    hi: 1.0,
                });
            }
        }

        // Bipolar tension: real singers swing both ways around neutral.
        if !(-1.0..=1.0).contains(&self.tension) {
            return Err(VocalParamsError::OutOfRange {
                field: "tension",
                value: self.tension,
                lo: -1.0,
                hi: 1.0,
            });
        }

        // Vibrato rate: classical / pop singers run 4-7 Hz; we accept
        // the broader 1-20 Hz band so users can dial extreme settings
        // without tripping the bound.
        if !(1.0..=20.0).contains(&self.vibrato_rate) {
            return Err(VocalParamsError::OutOfRange {
                field: "vibrato_rate",
                value: self.vibrato_rate,
                lo: 1.0,
                hi: 20.0,
            });
        }

        // Portamento: SVS pipeline applies it as a linear ramp; a zero
        // or negative value would invert the ramp direction.
        if !(0.0..=500.0).contains(&self.portamento_ms) {
            return Err(VocalParamsError::OutOfRange {
                field: "portamento_ms",
                value: self.portamento_ms,
                lo: 0.0,
                hi: 500.0,
            });
        }

        Ok(())
    }
}

fn default_vocal_style() -> VocalStyle {
    VocalStyle::PopBallad
}

fn default_vibrato_rate() -> f32 {
    5.0
}

fn default_tension() -> f32 {
    // Empirically the most-intelligible Lilia baseline (whisper STT
    // sweep of "the sun is shining bright" hit a perfect 1.00 with
    // tension=0.5 + both modulators=0.5). TIGER ignores this value
    // entirely (its acoustic model has no `tension` input), so
    // bumping the default doesn't affect TIGER projects.
    0.5
}

fn default_tension_velocity_amount() -> f32 {
    0.5
}

fn default_tension_contour_amount() -> f32 {
    0.5
}

fn default_portamento_ms() -> f32 {
    40.0
}

/// Seeded preview lines that mirror the prototype's draft.
fn default_draft() -> Vec<LyricLine> {
    vec![
        LyricLine {
            n: 1,
            rhyme: 'A',
            syllables: 11,
            text: "Glass hou·ses don't break, they just re·mem·ber".into(),
            locked: true,
        },
        LyricLine {
            n: 2,
            rhyme: 'B',
            syllables: 9,
            text: "ev·ry stone we threw on the way".into(),
            locked: false,
        },
        LyricLine {
            n: 3,
            rhyme: 'A',
            syllables: 11,
            text: "the way the light bends as you en·ter the room".into(),
            locked: false,
        },
        LyricLine {
            n: 4,
            rhyme: 'B',
            syllables: 8,
            text: "and ev·ery·thing I meant to say".into(),
            locked: false,
        },
    ]
}
