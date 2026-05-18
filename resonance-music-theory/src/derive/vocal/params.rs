//! Vocal-generator parameter enums + `VocalParams` struct + all the
//! `#[serde(default)]` helper fns. Pure data; no rng, no melody logic.

use serde::{Deserialize, Serialize};

use super::lyrics::LyricLine;

/// Lyrical mood preset. Drives the lyric generator's word choice and
/// chord-mood pairing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
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
    pub const ALL: [VocalMood; 6] = [
        VocalMood::Yearning,
        VocalMood::Defiant,
        VocalMood::Hopeful,
        VocalMood::Reflective,
        VocalMood::Joyful,
        VocalMood::Melancholy,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Lyrical point of view.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
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
    pub const ALL: [VocalPov; 5] = [
        VocalPov::FirstSingular,
        VocalPov::FirstPlural,
        VocalPov::SecondPerson,
        VocalPov::ThirdPerson,
        VocalPov::Narrator,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// End-rhyme scheme applied to the four (or N) generated lines.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
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
    pub const ALL: [VocalRhymeScheme; 5] = [
        VocalRhymeScheme::Aabb,
        VocalRhymeScheme::Abab,
        VocalRhymeScheme::Abcb,
        VocalRhymeScheme::Abba,
        VocalRhymeScheme::Free,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Voice type / tessitura preset.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
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
    pub const ALL: [VoiceType; 6] = [
        VoiceType::Soprano,
        VoiceType::MezzoSoprano,
        VoiceType::Alto,
        VoiceType::Tenor,
        VoiceType::Baritone,
        VoiceType::Bass,
    ];

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

    /// Singer this voice type historically mapped to. Used as the
    /// initial `VocalSinger` when a vocal lane is created so the default
    /// project sounds the way it always has — but the singer is
    /// independently editable afterward.
    pub fn default_singer(self) -> VocalSinger {
        match self {
            VoiceType::Soprano => VocalSinger::Glam,
            VoiceType::MezzoSoprano => VocalSinger::Fresh,
            VoiceType::Alto => VocalSinger::Disco,
            VoiceType::Tenor => VocalSinger::Royal,
            VoiceType::Baritone => VocalSinger::Electric,
            VoiceType::Bass => VocalSinger::Mystic,
        }
    }
}

/// Note → syllable mapping mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
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
    pub const ALL: [SyllableMode; 3] = [
        SyllableMode::Syllabic,
        SyllableMode::Mixed,
        SyllableMode::Melismatic,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Vocal phrase-contour family. Mirrors `ContourPreference` but kept
/// separate so the vocal rail can present its own glyph set (arch, rise,
/// fall, wave, flat) without polluting the instrument-melody enum.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum VocalContour {
    Arch,
    Rise,
    Fall,
    Wave,
    Flat,
}

impl VocalContour {
    pub const ALL: [VocalContour; 5] = [
        VocalContour::Arch,
        VocalContour::Rise,
        VocalContour::Fall,
        VocalContour::Wave,
        VocalContour::Flat,
    ];

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
    strum::Display, strum::IntoStaticStr, strum::EnumString,
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
    pub const ALL: [VocalStyle; 6] = [
        VocalStyle::PopBallad,
        VocalStyle::Conversational,
        VocalStyle::Hymnal,
        VocalStyle::Folk,
        VocalStyle::Anthemic,
        VocalStyle::Chant,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Top-level voicebank — the trained DiffSinger model that produces
/// the singing audio. Different voicebanks have completely different
/// vocal characters and own their own singer presets, so this is the
/// "outer" pick the user makes; `VocalSinger`/`VocalSingerMeiji` is the
/// per-voicebank inner pick.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum VocalVoicebank {
    /// TIGER (English DiffSinger v106) — 7 community speakers, the
    /// historic default since the SVS PoC.
    #[strum(serialize = "TIGER")]
    Tiger,
    /// LIEE Lilia (multi-language MM 2.8) — single-speaker voicebank
    /// with native English / Japanese / Tagalog / Korean / Spanish
    /// support. Sings in English through the same ARPAbet G2P we use
    /// for TIGER.
    Lilia,
    /// Gahata Meiji v160 (multi-language) — 4-mode voicebank with
    /// per-token language ids and `en/`-prefixed phonemes. Modes pick
    /// between standard / hunter / lilith / phantom character voices.
    Meiji,
}

impl VocalVoicebank {
    pub const ALL: [VocalVoicebank; 3] = [
        VocalVoicebank::Tiger,
        VocalVoicebank::Lilia,
        VocalVoicebank::Meiji,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Voicebank speaker preset. Picks which TIGER `tiger_*` singer drives
/// the SVS render. Decoupled from `VoiceType` so the user can have a
/// soprano-range melody sung by the disco speaker, etc. The default for
/// a new params block matches `VoiceType::default_singer` so projects
/// that never touch this field still get the historic mapping.
///
/// Singers are TIGER-specific. Lilia is single-speaker so the chip row
/// is hidden when Lilia is selected.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum VocalSinger {
    Glam,
    Fresh,
    Disco,
    Royal,
    Electric,
    Mystic,
    Vinyl,
}

impl VocalSinger {
    pub const ALL: [VocalSinger; 7] = [
        VocalSinger::Glam,
        VocalSinger::Fresh,
        VocalSinger::Disco,
        VocalSinger::Royal,
        VocalSinger::Electric,
        VocalSinger::Mystic,
        VocalSinger::Vinyl,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// TIGER speaker id this preset selects in the voicebank's `.emb`
    /// table. The pipeline looks the id up directly so renaming the
    /// presets here without touching the strings is safe.
    pub fn speaker_id(self) -> &'static str {
        match self {
            VocalSinger::Glam => "tiger_glam",
            VocalSinger::Fresh => "tiger_fresh",
            VocalSinger::Disco => "tiger_disco",
            VocalSinger::Royal => "tiger_royal",
            VocalSinger::Electric => "tiger_electric",
            VocalSinger::Mystic => "tiger_mystic",
            VocalSinger::Vinyl => "tiger_vinyl",
        }
    }
}

/// Meiji voicebank's singer modes. Distinct from `VocalSinger` because
/// the two voicebanks ship completely different presets: TIGER's
/// `tiger_*` are seven separate community singers, while Meiji's four
/// modes are character variants of the same base voice (Standard is
/// neutral, Hunter is strong, Lilith is mature, Phantom is whisper).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum VocalSingerMeiji {
    Standard,
    Hunter,
    Lilith,
    Phantom,
}

impl VocalSingerMeiji {
    pub const ALL: [VocalSingerMeiji; 4] = [
        VocalSingerMeiji::Standard,
        VocalSingerMeiji::Hunter,
        VocalSingerMeiji::Lilith,
        VocalSingerMeiji::Phantom,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Speaker id this preset selects in Meiji's `embeds/*.emb` table.
    /// Meiji's `dsconfig.yaml` lists speakers with the `embeds/` path
    /// prefix (e.g. `embeds/standard`); the pipeline matches on that
    /// exact string, so we include it here.
    pub fn speaker_id(self) -> &'static str {
        match self {
            VocalSingerMeiji::Standard => "embeds/standard",
            VocalSingerMeiji::Hunter => "embeds/hunter",
            VocalSingerMeiji::Lilith => "embeds/lilith",
            VocalSingerMeiji::Phantom => "embeds/phantom",
        }
    }
}

/// Vocal-line timbre preset.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::Display, strum::IntoStaticStr, strum::EnumString,
)]
pub enum VocalTimbre {
    Airy,
    Warm,
    Edged,
    Bright,
}

impl VocalTimbre {
    pub const ALL: [VocalTimbre; 4] = [
        VocalTimbre::Airy,
        VocalTimbre::Warm,
        VocalTimbre::Edged,
        VocalTimbre::Bright,
    ];

    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// All vocal-generator parameters surfaced in the right rail. Persists
/// alongside the rest of the lane generator configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub draft: Vec<LyricLine>,

    // ---- Melody ----
    pub voice: VoiceType,
    pub range: (u8, u8),
    /// Melodic style preset. Picks the per-syllable generator used by
    /// `derive_vocal`. Defaults to `PopBallad` (the legacy behavior) so
    /// older project files round-trip unchanged.
    #[serde(default = "default_vocal_style")]
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
    #[serde(default)]
    pub use_section_motif: bool,

    // ---- Voice & delivery ----
    pub timbre: VocalTimbre,
    pub vibrato: f32,
    /// Vibrato rate in Hz. Real singers run between roughly 4 Hz
    /// (slow / classical) and 7 Hz (operatic / pop pressure). Defaults
    /// to 5 Hz, the historic hardcoded value.
    #[serde(default = "default_vibrato_rate")]
    pub vibrato_rate: f32,
    /// Baseline voice tension in [-1.0, +1.0]. -1 = relaxed /
    /// breathy delivery, 0 = neutral, +1 = compressed / belted.
    /// Mapped to the SVS model's `tension` per-frame input on
    /// voicebanks that accept it (Lilia, Meiji); ignored for
    /// voicebanks that don't (TIGER). Per-frame modulators (see
    /// `tension_velocity_amount`, `tension_contour_amount`) add to
    /// this baseline so the actual tension can vary throughout the
    /// section.
    #[serde(default = "default_tension")]
    pub tension: f32,
    /// How much the per-syllable note velocity modulates tension
    /// dynamically. 0 = constant tension equal to the slider; 1 =
    /// strong syllables (accented beats) push tension up by ~+0.5,
    /// weak syllables push it down by ~-0.3 — re-creating the way
    /// real singers tense up on accents.
    #[serde(default = "default_tension_velocity_amount")]
    pub tension_velocity_amount: f32,
    /// How much the section's pitch contour modulates tension
    /// dynamically. 0 = no contour modulation; 1 = the highest notes
    /// of the section push tension up by ~+0.5 (singers belt at the
    /// top of their range), the lowest notes push it down by ~-0.5
    /// (more relaxed delivery on low notes).
    #[serde(default = "default_tension_contour_amount")]
    pub tension_contour_amount: f32,
    /// Pitch portamento (note-to-note glide) duration in milliseconds.
    /// Range ~10–200 ms. Lower = harder note transitions (tighter,
    /// more "snap"); higher = smoother glides (legato, scoopy). The
    /// SVS pipeline applies this as a linear ramp inserted before each
    /// pitch change. Defaults to 40 ms — the historic hardcoded value
    /// that matches the reference singing fixtures.
    #[serde(default = "default_portamento_ms")]
    pub portamento_ms: f32,
    pub articulation: f32,
    pub consonant_emphasis: f32,
    /// TIGER speaker preset. Decoupled from `voice` so the user can mix
    /// range and singer character. Defaults to `voice.default_singer()`
    /// when absent in older project files. Only meaningful when
    /// `voicebank == VocalVoicebank::Tiger`.
    #[serde(default = "default_singer")]
    pub singer: VocalSinger,
    /// Meiji singer mode. Only meaningful when
    /// `voicebank == VocalVoicebank::Meiji`. Stored alongside `singer`
    /// (rather than as part of an enum) so switching voicebanks
    /// preserves each side's last-chosen singer.
    #[serde(default = "default_singer_meiji")]
    pub singer_meiji: VocalSingerMeiji,
    /// Trained DiffSinger model used for the SVS render. Defaults to
    /// `Tiger` so projects that pre-date the multi-voicebank work pick
    /// up the historic singer; older JSON without the field falls back
    /// via `#[serde(default)]`.
    #[serde(default = "default_voicebank")]
    pub voicebank: VocalVoicebank,
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
            singer: voice.default_singer(),
            singer_meiji: default_singer_meiji(),
            voicebank: default_voicebank(),
        }
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

fn default_singer() -> VocalSinger {
    VoiceType::Alto.default_singer()
}

fn default_singer_meiji() -> VocalSingerMeiji {
    VocalSingerMeiji::Standard
}

fn default_voicebank() -> VocalVoicebank {
    VocalVoicebank::Tiger
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
