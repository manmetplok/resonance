//! Vocal generator: parameters, lyric corpus, and `derive_vocal` melody
//! synthesis.
//!
//! Lyric generation is template-based. A small mood-tagged corpus of
//! lines is bundled below; each line is pre-marked with a rhyme group
//! and a syllable count. The generator picks lines that satisfy the
//! requested mood, rhyme scheme, and syllable range, then writes them
//! back into `VocalParams::draft`.
//!
//! Melody generation walks the section's chord progression once per
//! syllable, picking a pitch that respects `range`, the active `contour`,
//! the `chord_tone_anchor` probability, the `leap_range` step-vs-leap
//! bias, and (optionally) the section's scale.

use serde::{Deserialize, Serialize};

use crate::rng::XorShift;
use crate::scale::Scale;

use super::motif_bass::chord_tones_in_register;
use super::{GeneratedNote, TimedChord};

/// Lyrical mood preset. Drives the lyric generator's word choice and
/// chord-mood pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        match self {
            VocalMood::Yearning => "Yearning",
            VocalMood::Defiant => "Defiant",
            VocalMood::Hopeful => "Hopeful",
            VocalMood::Reflective => "Reflective",
            VocalMood::Joyful => "Joyful",
            VocalMood::Melancholy => "Melancholy",
        }
    }
}

impl std::fmt::Display for VocalMood {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lyrical point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VocalPov {
    FirstSingular,
    FirstPlural,
    SecondPerson,
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
        match self {
            VocalPov::FirstSingular => "1st singular",
            VocalPov::FirstPlural => "1st plural",
            VocalPov::SecondPerson => "2nd person",
            VocalPov::ThirdPerson => "3rd person",
            VocalPov::Narrator => "Narrator",
        }
    }
}

impl std::fmt::Display for VocalPov {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// End-rhyme scheme applied to the four (or N) generated lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VocalRhymeScheme {
    Aabb,
    Abab,
    Abcb,
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
        match self {
            VocalRhymeScheme::Aabb => "AABB",
            VocalRhymeScheme::Abab => "ABAB",
            VocalRhymeScheme::Abcb => "ABCB",
            VocalRhymeScheme::Abba => "ABBA",
            VocalRhymeScheme::Free => "Free",
        }
    }
}

impl std::fmt::Display for VocalRhymeScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Voice type / tessitura preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceType {
    Soprano,
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
        match self {
            VoiceType::Soprano => "Soprano",
            VoiceType::MezzoSoprano => "Mezzo",
            VoiceType::Alto => "Alto",
            VoiceType::Tenor => "Tenor",
            VoiceType::Baritone => "Baritone",
            VoiceType::Bass => "Bass",
        }
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

impl std::fmt::Display for VoiceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Note → syllable mapping mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        match self {
            SyllableMode::Syllabic => "Syllabic",
            SyllableMode::Mixed => "Mixed",
            SyllableMode::Melismatic => "Melismatic",
        }
    }
}

impl std::fmt::Display for SyllableMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Vocal phrase-contour family. Mirrors `ContourPreference` but kept
/// separate so the vocal rail can present its own glyph set (arch, rise,
/// fall, wave, flat) without polluting the instrument-melody enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        match self {
            VocalContour::Arch => "Arch",
            VocalContour::Rise => "Rise",
            VocalContour::Fall => "Fall",
            VocalContour::Wave => "Wave",
            VocalContour::Flat => "Flat",
        }
    }
}

impl std::fmt::Display for VocalContour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VocalStyle {
    /// Stepwise, breath-driven, contour-shaped — Sting / Adele / Sade.
    /// The legacy default, kept for backwards compatibility.
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
        match self {
            VocalStyle::PopBallad => "Pop ballad",
            VocalStyle::Conversational => "Conversational",
            VocalStyle::Hymnal => "Hymnal",
            VocalStyle::Folk => "Folk",
            VocalStyle::Anthemic => "Anthemic",
            VocalStyle::Chant => "Chant",
        }
    }
}

impl std::fmt::Display for VocalStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Top-level voicebank — the trained DiffSinger model that produces
/// the singing audio. Different voicebanks have completely different
/// vocal characters and own their own singer presets, so this is the
/// "outer" pick the user makes; `VocalSinger`/`VocalSingerMeiji` is the
/// per-voicebank inner pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VocalVoicebank {
    /// TIGER (English DiffSinger v106) — 7 community speakers, the
    /// historic default since the SVS PoC.
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
        match self {
            VocalVoicebank::Tiger => "TIGER",
            VocalVoicebank::Lilia => "Lilia",
            VocalVoicebank::Meiji => "Meiji",
        }
    }
}

impl std::fmt::Display for VocalVoicebank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        match self {
            VocalSinger::Glam => "Glam",
            VocalSinger::Fresh => "Fresh",
            VocalSinger::Disco => "Disco",
            VocalSinger::Royal => "Royal",
            VocalSinger::Electric => "Electric",
            VocalSinger::Mystic => "Mystic",
            VocalSinger::Vinyl => "Vinyl",
        }
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

impl std::fmt::Display for VocalSinger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Meiji voicebank's singer modes. Distinct from `VocalSinger` because
/// the two voicebanks ship completely different presets: TIGER's
/// `tiger_*` are seven separate community singers, while Meiji's four
/// modes are character variants of the same base voice (Standard is
/// neutral, Hunter is strong, Lilith is mature, Phantom is whisper).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        match self {
            VocalSingerMeiji::Standard => "Standard",
            VocalSingerMeiji::Hunter => "Hunter",
            VocalSingerMeiji::Lilith => "Lilith",
            VocalSingerMeiji::Phantom => "Phantom",
        }
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

impl std::fmt::Display for VocalSingerMeiji {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Vocal-line timbre preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        match self {
            VocalTimbre::Airy => "Airy",
            VocalTimbre::Warm => "Warm",
            VocalTimbre::Edged => "Edged",
            VocalTimbre::Bright => "Bright",
        }
    }
}

impl std::fmt::Display for VocalTimbre {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One generated lyric line in the draft preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    /// 1-based line number.
    pub n: u8,
    /// Rhyme tag — 'A', 'B', 'C', etc. matching the chosen scheme.
    pub rhyme: char,
    /// Syllable count.
    pub syllables: u8,
    /// The line itself, with `·` used as a syllable separator (matches
    /// the prototype's typographic convention).
    pub text: String,
    /// Locked lines are not replaced by the next re-roll.
    pub locked: bool,
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

// ===========================================================================
// Lyric corpus + generator
// ===========================================================================

/// One template line in the bundled lyric corpus.
struct CorpusLine {
    mood: VocalMood,
    /// Rhyme bucket — lines that share this tag end on the same vowel
    /// sound. Same tag = same rhyme.
    rhyme: u8,
    /// Total syllable count of `text` (the syllable separator `·` is
    /// significant — `text` is already pre-broken).
    syllables: u8,
    /// Line, with `·` between syllables.
    text: &'static str,
}

/// Tiny mood-tagged corpus. Each line is pre-syllabified and pre-tagged
/// with a rhyme bucket so the generator can build any A/B/C scheme by
/// matching tags rather than running a phonetic rhyme matcher at runtime.
///
/// Rhyme bucket numbering is local and only meaningful within this file
/// — buckets in different moods aren't interchangeable.
const CORPUS: &[CorpusLine] = &[
    // ---- Yearning (rhyme 1: -ay/-ey, 2: -one/-ome, 3: -ember/-er) ----
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 9, text: "I hold the days I can·not say" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 10, text: "the morn·ing leaves us no·thing to weigh" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 8, text: "ev·ery·thing I meant to say" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 1, syllables: 9, text: "an emp·ty house an emp·ty day" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 9, text: "ev·ry stone we threw on the way" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 10, text: "we walk the hall·ways all a·lone" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 9, text: "a qui·et house that feels like home" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 2, syllables: 9, text: "I made a bed of glass and stone" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 3, syllables: 11, text: "Glass hou·ses don't break, they just re·mem·ber" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 3, syllables: 11, text: "the way the light bends as you en·ter the room" },
    CorpusLine { mood: VocalMood::Yearning, rhyme: 3, syllables: 10, text: "a name I have not learned to fore·get" },

    // ---- Defiant (1: -own, 2: -ire/-ind, 3: -ash/-ack) ----
    CorpusLine { mood: VocalMood::Defiant, rhyme: 1, syllables: 8, text: "I won't lay this an·ger down" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 1, syllables: 9, text: "I am not the one who broke the crown" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 1, syllables: 8, text: "burn the maps that brought us down" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 2, syllables: 9, text: "I walk a·gainst the lev·el wind" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 2, syllables: 9, text: "I will not bend, I will not bind" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 2, syllables: 10, text: "the room is full of bor·rowed fire" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 3, syllables: 8, text: "I let the qui·et turn to ash" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 3, syllables: 9, text: "I'll meet your an·ger with my own back" },
    CorpusLine { mood: VocalMood::Defiant, rhyme: 3, syllables: 8, text: "noth·ing breaks me, noth·ing cracks" },

    // ---- Hopeful (1: -ight, 2: -orn/-orning, 3: -ound) ----
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 1, syllables: 9, text: "and ev·ery cor·ner finds the light" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 1, syllables: 8, text: "a thou·sand suns be·hind the night" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 1, syllables: 9, text: "I learn to read the dark·er signs" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 2, syllables: 9, text: "I'll meet you in the qui·et morn·ing" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 2, syllables: 9, text: "the world is gen·tle when it's born" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 2, syllables: 10, text: "a low and stead·y wind across the lawn" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 3, syllables: 8, text: "ev·ry·thing I lost is found" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 3, syllables: 9, text: "I press my ear against the ground" },
    CorpusLine { mood: VocalMood::Hopeful, rhyme: 3, syllables: 9, text: "the soft and pa·tient turn·ing sound" },

    // ---- Reflective (1: -ow/-ow, 2: -ile, 3: -ear/-eel) ----
    CorpusLine { mood: VocalMood::Reflective, rhyme: 1, syllables: 8, text: "I watch the riv·er find its flow" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 1, syllables: 9, text: "we used to know each oth·er though" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 1, syllables: 10, text: "I car·ry on the way the rains do go" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 2, syllables: 9, text: "I count the lights, I sit a while" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 2, syllables: 9, text: "an old, fa·mil·iar morn·ing smile" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 2, syllables: 8, text: "the streets that ran on for a mile" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 3, syllables: 9, text: "I think of you and feel quite near" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 3, syllables: 8, text: "I bend my head and hear, and hear" },
    CorpusLine { mood: VocalMood::Reflective, rhyme: 3, syllables: 9, text: "ev·ery still·ness makes it clear" },

    // ---- Joyful (1: -ee, 2: -ay, 3: -ound) ----
    CorpusLine { mood: VocalMood::Joyful, rhyme: 1, syllables: 8, text: "you laugh, the whole room laughs with me" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 1, syllables: 9, text: "we run and run, the world goes free" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 1, syllables: 9, text: "I read the sky like po·et·ry" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 2, syllables: 8, text: "we won't be quiet, won't go a·way" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 2, syllables: 9, text: "the kind of morn·ing made for play" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 2, syllables: 8, text: "the kind of word·s I want to say" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 3, syllables: 8, text: "your laugh·ter spins us all around" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 3, syllables: 9, text: "we shake the win·dows with the sound" },
    CorpusLine { mood: VocalMood::Joyful, rhyme: 3, syllables: 9, text: "the kind of joy that won't stay bound" },

    // ---- Melancholy (1: -ain, 2: -ear/-ere, 3: -one) ----
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 1, syllables: 9, text: "I think of you and feel the rain" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 1, syllables: 8, text: "the qui·et stays, the streets re·main" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 1, syllables: 9, text: "the win·ter holds its old re·frain" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 2, syllables: 8, text: "I should have brought you clos·er here" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 2, syllables: 9, text: "you said the words but I'd not hear" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 2, syllables: 9, text: "the dis·tance be·tween then and here" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 3, syllables: 8, text: "I count the things that I have done" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 3, syllables: 9, text: "I leave the house and I am gone" },
    CorpusLine { mood: VocalMood::Melancholy, rhyme: 3, syllables: 9, text: "the long, the qui·et, the un·done" },
];

/// Pattern of rhyme keys for a given scheme. Letters share keys; "free"
/// returns an empty pattern (no rhyme constraint).
fn rhyme_pattern(scheme: VocalRhymeScheme) -> &'static [u8] {
    match scheme {
        VocalRhymeScheme::Aabb => &[0, 0, 1, 1],
        VocalRhymeScheme::Abab => &[0, 1, 0, 1],
        VocalRhymeScheme::Abcb => &[0, 1, 2, 1],
        VocalRhymeScheme::Abba => &[0, 1, 1, 0],
        VocalRhymeScheme::Free => &[],
    }
}

/// Generate (or regenerate) a `params.draft` of lyric lines satisfying
/// the requested mood, rhyme scheme, line count, and syllable range.
///
/// Locked lines keep their current text/rhyme/syllables; the rhyme
/// pattern is *anchored* to any locked line so re-rolls preserve the
/// established A/B groupings.
pub fn generate_lyrics(params: &VocalParams, seed: u64) -> Vec<LyricLine> {
    let mut rng = XorShift::new(seed.max(1));
    let lines = params.lines.max(1) as usize;
    let pattern = rhyme_pattern(params.rhyme);

    // Letter assigned to each unique rhyme key (0 -> 'A', 1 -> 'B', ...).
    let letter_for = |k: u8| -> char { (b'A' + (k % 26)) as char };

    // Filter corpus by mood. Fall back to all moods if the bucket is empty.
    let mood_pool: Vec<&CorpusLine> =
        CORPUS.iter().filter(|l| l.mood == params.mood).collect();
    let fallback_pool: Vec<&CorpusLine> = CORPUS.iter().collect();
    let active_pool = if mood_pool.is_empty() {
        &fallback_pool
    } else {
        &mood_pool
    };

    // Distinct rhyme keys present in the active pool.
    let mut available_keys: Vec<u8> = active_pool.iter().map(|l| l.rhyme).collect();
    available_keys.sort_unstable();
    available_keys.dedup();
    if available_keys.is_empty() {
        return params.draft.clone();
    }

    // Anchor any rhyme-pattern slot whose corresponding line is locked
    // to the locked line's rhyme bucket. The bucket is recovered by
    // matching the locked line's rhyme letter back through the pattern.
    // Pattern entries with no corresponding locked line get a freshly
    // picked bucket (without repeating already-used buckets when
    // possible).
    let mut pattern_keys: std::collections::HashMap<u8, u8> = Default::default();
    if !pattern.is_empty() {
        for (i, slot) in pattern.iter().enumerate().take(lines) {
            if let Some(line) = params.draft.get(i) {
                if line.locked {
                    // Recover the bucket from the locked line's text by
                    // re-matching it in the corpus.
                    let bucket = CORPUS
                        .iter()
                        .find(|c| c.text == line.text)
                        .map(|c| c.rhyme)
                        .unwrap_or(*slot);
                    pattern_keys.insert(*slot, bucket);
                }
            }
        }
        for slot in pattern.iter().take(lines) {
            if pattern_keys.contains_key(slot) {
                continue;
            }
            let used: std::collections::HashSet<u8> =
                pattern_keys.values().copied().collect();
            // Pick an unused key when possible.
            let unused: Vec<u8> = available_keys
                .iter()
                .copied()
                .filter(|k| !used.contains(k))
                .collect();
            let key = if !unused.is_empty() {
                unused[rng.next_range(unused.len())]
            } else {
                available_keys[rng.next_range(available_keys.len())]
            };
            pattern_keys.insert(*slot, key);
        }
    }

    // For each output line: pick from the pool that matches its bucket
    // (or any line if scheme is Free), syllable count in range.
    let mut out = Vec::with_capacity(lines);
    let mut used_texts: std::collections::HashSet<String> = Default::default();
    for i in 0..lines {
        let n = (i + 1) as u8;

        // Preserve a locked line if present.
        if let Some(existing) = params.draft.get(i) {
            if existing.locked {
                used_texts.insert(existing.text.clone());
                out.push(existing.clone());
                continue;
            }
        }

        let bucket = if pattern.is_empty() {
            // Free scheme — any rhyme bucket goes.
            available_keys[rng.next_range(available_keys.len())]
        } else {
            *pattern_keys
                .get(&pattern[i % pattern.len()])
                .unwrap_or(&available_keys[0])
        };

        // Candidate pool: bucket match + syllable range + not already used.
        let mut candidates: Vec<&CorpusLine> = active_pool
            .iter()
            .filter(|l| {
                l.rhyme == bucket
                    && l.syllables >= params.syllables_min
                    && l.syllables <= params.syllables_max
                    && !used_texts.contains(l.text)
            })
            .copied()
            .collect();
        // Fall back to bucket match without syllable constraint, then
        // any bucket. Keeps the generator from returning the placeholder
        // line on niche corpora.
        if candidates.is_empty() {
            candidates = active_pool
                .iter()
                .filter(|l| l.rhyme == bucket && !used_texts.contains(l.text))
                .copied()
                .collect();
        }
        if candidates.is_empty() {
            candidates = active_pool
                .iter()
                .filter(|l| !used_texts.contains(l.text))
                .copied()
                .collect();
        }
        if candidates.is_empty() {
            // Should be unreachable given we always have one corpus
            // line; emit a sentinel so the UI still has something.
            out.push(LyricLine {
                n,
                rhyme: letter_for(bucket),
                syllables: 0,
                text: String::from("\u{2014}"),
                locked: false,
            });
            continue;
        }
        let pick = candidates[rng.next_range(candidates.len())];
        used_texts.insert(pick.text.to_string());

        let letter = if pattern.is_empty() {
            'F' // "Free"
        } else {
            letter_for(pattern[i % pattern.len()])
        };

        out.push(LyricLine {
            n,
            rhyme: letter,
            syllables: pick.syllables,
            text: pick.text.to_string(),
            locked: false,
        });
    }
    out
}

// ===========================================================================
// Melody generator
// ===========================================================================

/// Strip the syllable separator and count syllables in a lyric line. A
/// fallback for cases where `LyricLine::syllables` is 0.
pub fn count_syllables(text: &str) -> u32 {
    let dot_count = text.matches('\u{00B7}').count() as u32;
    // `n syllables = dot_count + word_count` is a reasonable approximation
    // for already-broken text; we add the dots to the word count.
    let word_count = text.split_whitespace().count() as u32;
    (dot_count + word_count).max(1)
}

/// Map a normalised time `t ∈ [0, 1]` to a unit pitch height according
/// to a contour shape. 0.0 = bottom of the range, 1.0 = top.
fn contour_height(contour: VocalContour, t: f32) -> f32 {
    use std::f32::consts::PI;
    let t = t.clamp(0.0, 1.0);
    match contour {
        VocalContour::Arch => (PI * t).sin().clamp(0.0, 1.0),
        VocalContour::Rise => 0.15 + 0.80 * t,
        VocalContour::Fall => 0.95 - 0.80 * t,
        VocalContour::Wave => 0.5 + 0.4 * (1.5 * 2.0 * PI * t).sin(),
        VocalContour::Flat => 0.5 + 0.05 * (8.0 * t).sin(),
    }
}

/// Snap a MIDI note to the nearest scale tone, scanning outward up to
/// 6 semitones. Falls back to the input when no scale tone is reachable.
fn snap_to_scale(note: u8, scale: Option<Scale>, lo: u8, hi: u8) -> u8 {
    let Some(scale) = scale else { return note };
    for d in 0..=6i16 {
        for &sign in &[1i16, -1] {
            let candidate = note as i16 + d * sign;
            if (lo as i16..=hi as i16).contains(&candidate)
                && scale.contains(candidate as u8)
            {
                return candidate as u8;
            }
        }
    }
    note
}

/// Find the chord active at a given beat. Returns the last chord whose
/// start ≤ beat. If none match (e.g. beat is before the first chord),
/// returns the first chord.
fn chord_at_beat(chords: &[TimedChord], beat: u32) -> Option<&TimedChord> {
    let mut active = chords.first();
    for c in chords {
        if c.start_beat <= beat {
            active = Some(c);
        }
    }
    active
}

/// Total beat span covered by the chord list — from beat 0 to the
/// furthest chord end.
fn total_beats(chords: &[TimedChord]) -> u32 {
    chords
        .iter()
        .map(|c| c.start_beat + c.duration_beats)
        .max()
        .unwrap_or(0)
}

/// Derive MIDI notes for a vocal line. Dispatches to the per-style
/// generator chosen by `params.style`; each style is its own per-
/// syllable walk with distinct pitch and rhythm strategies.
///
/// All styles share the same one-note-per-syllable invariant — the SVS
/// pipeline indexes syllable phonemes by note position, so a melismatic
/// style would need pipeline changes too (see vocal_svs.rs).
pub fn derive_vocal(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    // Default to 4/4 when the caller doesn't know the meter — keeps
    // older / test call sites working unchanged.
    derive_vocal_with_meter(chords, params, ticks_per_beat, 4, seed)
}

/// Same as [`derive_vocal`] but also takes the section's
/// `beats_per_bar` so the generators know where bar boundaries fall.
/// 4 = 4/4, 3 = 3/4 / waltz, 6 = 6/8 (compound), 2 = 2/4 / cut. The
/// app should pass its current transport time-signature numerator
/// here. Affects beat-strength accents and per-line phrase-start
/// offsets (pickup, anacrusis, off-beat starts).
pub fn derive_vocal_with_meter(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    beats_per_bar: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    derive_vocal_with_motif(chords, params, ticks_per_beat, beats_per_bar, None, seed)
}

/// Same as [`derive_vocal_with_meter`] but optionally re-skins the
/// per-syllable pitches with a shared motif. Pass `motif_intervals` as
/// a non-empty slice of signed semitone offsets relative to the chord
/// root; the style's rhythm + dynamics still drive the surface, only
/// pitches are overridden. The final syllable of each line keeps its
/// style cadence landing so phrases still resolve.
///
/// When `params.use_section_motif` is `false` or `motif_intervals` is
/// empty, the motif is ignored — equivalent to plain
/// [`derive_vocal_with_meter`].
pub fn derive_vocal_with_motif(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    beats_per_bar: u32,
    motif_intervals: Option<&[i8]>,
    seed: u64,
) -> Vec<GeneratedNote> {
    let ctx = match VocalContext::build(chords, params, ticks_per_beat, beats_per_bar, seed) {
        Some(ctx) => ctx,
        None => return Vec::new(),
    };
    let mut notes = derive_with_profile(&ctx);
    if params.use_section_motif {
        if let Some(intervals) = motif_intervals {
            if !intervals.is_empty() {
                apply_motif_pitches(
                    &mut notes,
                    intervals,
                    &ctx.line_syllables,
                    ctx.chords,
                    ctx.section_beats,
                    ctx.scale,
                    (ctx.lo, ctx.hi),
                    ctx.tpb,
                );
            }
        }
    }
    enforce_no_overlap(&mut notes, ctx.tpb);
    notes
}

/// Replace each note's pitch with a motif-derived pitch (chord root in
/// the lane register + signed motif interval, snapped to scale and
/// clamped within an octave of the previous pitch). The terminal note
/// of every line keeps its style cadence landing so phrases still
/// resolve.
#[allow(clippy::too_many_arguments)]
fn apply_motif_pitches(
    notes: &mut [GeneratedNote],
    motif_intervals: &[i8],
    line_syllables: &[u32],
    chords: &[TimedChord],
    section_beats: u32,
    scale: Option<Scale>,
    range: (u8, u8),
    tpb: u64,
) {
    if motif_intervals.is_empty() || notes.is_empty() {
        return;
    }
    let (lo, hi) = range;
    let centre = ((lo as u16 + hi as u16) / 2) as u8;
    let mut prev_pitch = snap_to_scale(centre, scale, lo, hi);
    let mut note_idx = 0usize;

    for (line_idx, &line_syl) in line_syllables.iter().enumerate() {
        if line_syl == 0 {
            continue;
        }
        let line_note_count = (line_syl as usize).min(notes.len() - note_idx);
        if line_note_count == 0 {
            break;
        }
        for s in 0..line_note_count {
            let n = &mut notes[note_idx + s];
            let beat = (n.start_tick / tpb) as u32;
            let beat_clamped = beat.min(section_beats.saturating_sub(1));
            let chord = chord_at_beat(chords, beat_clamped);
            let is_final = s + 1 == line_note_count;

            let raw = if is_final {
                cadence_pitch(phrase_role(line_idx), chord, scale, prev_pitch, range)
                    .unwrap_or_else(|| {
                        let interval = motif_intervals[s % motif_intervals.len()];
                        motif_pitch(interval, chord, lo, hi, prev_pitch, scale)
                    })
            } else {
                let interval = motif_intervals[s % motif_intervals.len()];
                motif_pitch(interval, chord, lo, hi, prev_pitch, scale)
            };
            let pitch = cap_interval(prev_pitch, raw, lo, hi, scale);
            n.note = pitch;
            prev_pitch = pitch;
        }
        note_idx += line_note_count;
        if note_idx >= notes.len() {
            break;
        }
    }
}

/// Anchor pitch + signed motif interval, snapped to scale and range.
/// The anchor is the chord root in the lane register nearest to the
/// previous pitch (so motif transposes follow the chord progression
/// and the line stays in tessitura).
fn motif_pitch(
    interval: i8,
    chord: Option<&TimedChord>,
    lo: u8,
    hi: u8,
    prev: u8,
    scale: Option<Scale>,
) -> u8 {
    let anchor = chord
        .map(|c| {
            let root_pc = c.chord.root.to_semitone() as i16;
            // Find the in-range MIDI note nearest `prev` whose pitch
            // class equals the chord root.
            (lo..=hi)
                .filter(|p| (*p as i16 - root_pc).rem_euclid(12) == 0)
                .min_by_key(|p| (*p as i16 - prev as i16).abs())
                .unwrap_or(prev)
        })
        .unwrap_or(prev);
    let candidate = (anchor as i16 + interval as i16).clamp(lo as i16, hi as i16) as u8;
    snap_to_scale(candidate, scale, lo, hi)
}

/// Group `notes` (one per syllable, in lyric order) into per-line
/// `(start_tick, end_tick)` phrase intervals using `params.draft` to
/// recover the lyric line boundaries. Each interval's start is the
/// earliest onset of any note in the line and its end is the latest
/// note's `start_tick + duration_ticks`. Lines with no syllables are
/// skipped.
///
/// Used by `MelodyParams::fill_vocal_gaps`: the synth fill needs to
/// know where the actual sung phrases sit, and the lyric line is the
/// authoritative phrase unit. Time-gap heuristics fail because the
/// vocal generator's `phrase_start_offset` can pull successive lines
/// into each other, leaving only a few-tick gap between them.
pub fn vocal_phrase_spans(
    notes: &[GeneratedNote],
    params: &VocalParams,
) -> Vec<(u64, u64)> {
    let line_syl: Vec<u32> = params
        .draft
        .iter()
        .map(|l| count_syllables(&l.text))
        .collect();
    let mut out = Vec::with_capacity(line_syl.len());
    let mut cursor = 0usize;
    for &n_syl in &line_syl {
        let n = (n_syl as usize).min(notes.len().saturating_sub(cursor));
        if n == 0 {
            continue;
        }
        let slice = &notes[cursor..cursor + n];
        let start = slice.iter().map(|x| x.start_tick).min().unwrap_or(0);
        let end = slice
            .iter()
            .map(|x| x.start_tick + x.duration_ticks)
            .max()
            .unwrap_or(start);
        out.push((start, end));
        cursor += n;
    }
    out
}

/// Final pass: each note's `start_tick + duration_ticks` must not
/// exceed the next note's `start_tick`. The `phrase_start_offset`
/// (negative pickup / anacrusis) can shift line N+1 to start before
/// line N's terminal sustain ends, which previously surfaced as
/// "doubled" notes — the SVS pipeline indexes phonemes by note slot,
/// so an overlap means two syllables claim the same time window and
/// the second one's pitch fights the first's tail.
///
/// We compute the time order via a permutation (instead of sorting
/// the notes themselves) so the original lyric order survives — the
/// app's `vocal_phrase_spans` walks notes in lyric order to recover
/// per-line phrase intervals, and a sort would mix lines together
/// when `phrase_start_offset` shifts a later line back into an
/// earlier one's tail. We trim each note's duration to leave at
/// least `tpb / 16` (a 64th note) of silence into the next-in-time
/// note's onset.
fn enforce_no_overlap(notes: &mut [GeneratedNote], tpb: u64) {
    if notes.len() < 2 {
        return;
    }
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|&i| notes[i].start_tick);
    let min_gap = (tpb / 16).max(1);
    for w in order.windows(2) {
        let (cur_idx, next_idx) = (w[0], w[1]);
        let next_start = notes[next_idx].start_tick;
        let cur_start = notes[cur_idx].start_tick;
        let cur_end = cur_start + notes[cur_idx].duration_ticks;
        if cur_end + min_gap > next_start {
            let new_dur = next_start.saturating_sub(cur_start).saturating_sub(min_gap);
            notes[cur_idx].duration_ticks = new_dur.max(1);
        }
    }
}

/// Bundle of validated inputs every per-style generator needs.
/// Building it once up front lets each style stay focused on the
/// musical decisions instead of repeating the same boilerplate.
struct VocalContext<'a> {
    chords: &'a [TimedChord],
    params: &'a VocalParams,
    tpb: u64,
    section_beats: u32,
    /// Section time-signature numerator. 4 = 4/4 (default), 3 = 3/4,
    /// 6 = 6/8, 2 = 2/4. Drives `beat_strength` and the per-line
    /// `phrase_start_offset`.
    beats_per_bar: u32,
    lo: u8,
    hi: u8,
    /// One entry per draft line, in order, holding the line's
    /// mechanical syllable count (matches what the SVS pipeline walks).
    line_syllables: Vec<u32>,
    total_syl: u32,
    seed: u64,
    scale: Option<Scale>,
}

impl<'a> VocalContext<'a> {
    fn build(
        chords: &'a [TimedChord],
        params: &'a VocalParams,
        ticks_per_beat: u32,
        beats_per_bar: u32,
        seed: u64,
    ) -> Option<Self> {
        if chords.is_empty() || params.draft.is_empty() {
            return None;
        }
        let section_beats = total_beats(chords);
        if section_beats == 0 {
            return None;
        }
        let (lo, hi) = params.range;
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        if lo == hi {
            return None;
        }
        // We use `count_syllables` (mechanical count of `·`-separators
        // + whitespace) rather than the corpus's `syllables` field
        // because the SVS render path walks words/syllables with the
        // same mechanical count — trusting the corpus field can leave
        // dangling notes that surface as phantom "ah" syllables.
        let line_syllables: Vec<u32> = params
            .draft
            .iter()
            .map(|l| count_syllables(&l.text))
            .collect();
        let total_syl: u32 = line_syllables.iter().sum();
        if total_syl == 0 {
            return None;
        }
        let scale = if params.stay_in_scale {
            scale_from_chords(chords)
        } else {
            None
        };
        Some(Self {
            chords,
            params,
            tpb: ticks_per_beat as u64,
            section_beats,
            beats_per_bar: beats_per_bar.max(1),
            lo,
            hi,
            line_syllables,
            total_syl,
            seed,
            scale,
        })
    }
}

/// Maximum interval the SVS model can cleanly render between adjacent
/// syllables. Bigger jumps surface as glitched audio, so every style
/// caps its per-syllable interval at this value.
const MAX_INTERVAL: i16 = 9;

/// Cap a candidate pitch so it sits within `MAX_INTERVAL` semitones of
/// the previous pitch. When clamped, snap back into the scale so we
/// stay musical.
fn cap_interval(prev: u8, candidate: u8, lo: u8, hi: u8, scale: Option<Scale>) -> u8 {
    let delta = candidate as i16 - prev as i16;
    if delta.abs() <= MAX_INTERVAL {
        return candidate;
    }
    let dir = delta.signum();
    let capped = (prev as i16 + dir * MAX_INTERVAL).clamp(lo as i16, hi as i16) as u8;
    snap_to_scale(capped, scale, lo, hi)
}

/// Pentatonic filter: true when `note` is a "safe" pentatonic degree of
/// `scale`. Drops the 4th and 7th in major-ish modes and the 2nd and 6th
/// in minor-ish modes. Used by the Folk style.
fn is_pentatonic(note: u8, scale: Scale) -> bool {
    use crate::scale::Mode;
    let semitone = note % 12;
    let root = scale.root.to_semitone();
    let degree = (semitone + 12 - root) % 12;
    let drop: &[u8] = match scale.mode {
        Mode::Minor | Mode::Phrygian | Mode::Locrian | Mode::HarmonicMinor => &[2, 8], // omit 2nd, b6/6
        _ => &[5, 11], // omit 4, 7 (and b7 for mixolydian close enough)
    };
    !drop.contains(&degree)
}

/// Snap to the nearest pentatonic note within range. Falls back to a
/// plain scale snap, then to the input.
fn snap_to_pentatonic(note: u8, scale: Option<Scale>, lo: u8, hi: u8) -> u8 {
    let Some(scale) = scale else { return note };
    for d in 0..=6i16 {
        for &sign in &[1i16, -1] {
            let candidate = note as i16 + d * sign;
            if (lo as i16..=hi as i16).contains(&candidate)
                && scale.contains(candidate as u8)
                && is_pentatonic(candidate as u8, scale)
            {
                return candidate as u8;
            }
        }
    }
    snap_to_scale(note, Some(scale), lo, hi)
}

/// Pick the chord tone in `range` closest to `target`. Returns `None`
/// when the chord has no tones in the requested range.
fn chord_tone_nearest(chord: super::super::chord::Chord, range: (u8, u8), target: u8) -> Option<u8> {
    let tones = chord_tones_in_register(chord, range);
    tones
        .into_iter()
        .min_by_key(|t| (*t as i16 - target as i16).abs())
}

/// Phrase-arch envelope: returns a 0..1 multiplier shaped like a real
/// vocal phrase — gentle build into a peak around 65 % of the line,
/// then a softer fall-off. Used by every style's velocity formula
/// to add line-shape dynamics instead of every syllable sitting at
/// the same level.
fn phrase_arch(progress_in_line: f32) -> f32 {
    let p = progress_in_line.clamp(0.0, 1.0);
    let peak = 0.65;
    let v = if p <= peak {
        // Smooth attack: square-ease so opening syllables aren't
        // identical in level.
        (p / peak).powf(0.7)
    } else {
        // Gentler tail than attack so the line release feels natural.
        1.0 - 0.55 * ((p - peak) / (1.0 - peak)).powf(1.2)
    };
    v.clamp(0.0, 1.0)
}

/// Beat-of-bar accent strength in [0, 1]. Drives velocity accents
/// and some pitch decisions (chord-tone landing on strong beats).
///
/// Meter awareness: 4/4 puts the strongest accent on beat 1 with a
/// secondary on beat 3; 3/4 has a single strong on beat 1 and weak
/// 2 + 3; 6/8 (compound time) has primary on beat 1 and secondary
/// on beat 4 of the eighth-count, which translates to beat 0 + 1.5
/// in quarter-note time. We keep beats integers by approximating
/// 6/8 as a 6-beat cycle in eighth notes — callers that pass
/// beats_per_bar=6 get the compound feel.
fn beat_strength(beat: u32, beats_per_bar: u32) -> f32 {
    let in_bar = beat % beats_per_bar.max(1);
    match beats_per_bar {
        // 6/8 compound: strong on 1 and 4 of the 6-eighth cycle.
        6 => match in_bar {
            0 => 1.0,
            3 => 0.70,
            _ => 0.30,
        },
        // 3/4 / waltz: strong only on 1.
        3 => match in_bar {
            0 => 1.0,
            _ => 0.30,
        },
        // 2/4 / cut time: 1 strong, 2 weak.
        2 => match in_bar {
            0 => 1.0,
            _ => 0.35,
        },
        // 4/4 default (and any other meter we treat as duple).
        _ => match in_bar {
            0 => 1.0,
            x if x == beats_per_bar / 2 => 0.65,
            _ => 0.30,
        },
    }
}

/// Per-syllable trim multiplier — controls what fraction of the
/// rigid `beat_step` slot each note actually fills. Variation comes
/// from three sources:
///   - Bar position: strong beats hold longer (long note feel),
///     weak beats are shorter (creates a gap after).
///   - Style "energy": pop ballad uses gentler variation, chant
///     uses sharper longs/shorts, conversational has irregular
///     bursts.
///   - Jitter: small per-syllable randomness so consecutive notes
///     aren't carbon copies.
///
/// `base_trim` is the style's default (e.g. 0.66 for PopBallad);
/// `range` is the half-width of the variation envelope. Returns a
/// trim in [0.30, 0.95].
fn rhythm_trim(
    rng: &mut XorShift,
    base_trim: f32,
    beat: u32,
    beats_per_bar: u32,
    range: f32,
) -> f32 {
    let strength = beat_strength(beat, beats_per_bar); // 0..1
    // Strong beats lengthen toward base + range; weak beats shorten
    // toward base - range. Adds an audible swing without changing
    // syllable positions on the grid.
    let bias = (strength - 0.5) * 2.0 * range;
    let jitter = (rng.next_f32() - 0.5) * 0.08;
    (base_trim + bias + jitter).clamp(0.30, 0.95)
}

/// Duration of the final syllable of a line in beats. Replaces the
/// "fill the breath gap" math that used to hand the last note a
/// duration up to 4× longer than the rest of the line — that hang
/// reads as a mistake, not a held cadence note. Capped at 1.4× the
/// regular beat-step so the final note feels intentional without
/// dragging into the next phrase.
///
/// Note: an even briefer rest is still added by `enforce_no_overlap`
/// at the very end (one 64th-note gap), so the SVS pipeline always
/// sees a clean boundary into the next line.
fn terminal_dur_beats(beat_step: f32, articulation: f32) -> f32 {
    let trim = 0.98 - 0.48 * articulation.clamp(0.0, 1.0);
    let normal = beat_step * trim;
    // Held but not absurd: 1.4x the regular note.
    let held = beat_step * 1.4;
    held.max(normal)
}

/// Phrase-role classification for one line of lyrics. Antecedent
/// lines (0 + 2 in a 4-line block) end "open" — on a scale degree
/// that asks for more (2, 4, or 7). Consequent lines (1 + 3) end
/// "closed" — on the tonic (1), 3rd, or 5th. Drives where we land
/// the cadence pitch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhraseRole {
    Antecedent,
    Consequent,
}

fn phrase_role(line_idx: usize) -> PhraseRole {
    if line_idx % 2 == 1 {
        PhraseRole::Consequent
    } else {
        PhraseRole::Antecedent
    }
}

/// Pick a "good" cadence pitch for the final syllable of a line.
/// Lands on chord-tone scale degrees per `phrase_role`:
///   - Consequent → root / 3rd / 5th of the active chord (closed
///     feel) with a strong root preference.
///   - Antecedent → 2nd / 4th / 7th of the section's scale (open
///     feel — asks the next line to resolve).
///
/// Falls back to the nearest chord tone if no scale-degree match is
/// reachable in range. The picked pitch is also clamped within an
/// octave of `prev_pitch` so the cadence doesn't leap.
fn cadence_pitch(
    role: PhraseRole,
    chord: Option<&TimedChord>,
    scale: Option<Scale>,
    prev_pitch: u8,
    range: (u8, u8),
) -> Option<u8> {
    use crate::scale::Mode;
    let (lo, hi) = range;
    let chord = chord?;
    match role {
        PhraseRole::Consequent => {
            // Closed: prefer root, then 3rd, then 5th.
            let tones = chord_tones_in_register(chord.chord, (lo, hi));
            if tones.is_empty() {
                return None;
            }
            let root_pc = chord.chord.root.to_semitone();
            // Find note within an octave of prev_pitch that's closest
            // to the chord root (priority for the strongest landing).
            let candidate = tones
                .iter()
                .filter(|t| (**t as i16 - prev_pitch as i16).abs() <= 12)
                .min_by_key(|t| {
                    let pc = (*t % 12) as i32;
                    let root_dist = ((pc - root_pc as i32).abs()).min(12 - (pc - root_pc as i32).abs());
                    let pitch_dist = (**t as i16 - prev_pitch as i16).abs() as i32;
                    // Multiply root_dist by 1000 so it dominates.
                    root_dist * 1000 + pitch_dist
                });
            candidate.copied().or_else(|| tones.iter().min_by_key(|t| (**t as i16 - prev_pitch as i16).abs()).copied())
        }
        PhraseRole::Antecedent => {
            // Open: prefer 2nd / 4th / 7th of the active scale.
            let scale = scale?;
            let mode_intervals = scale.mode.intervals();
            let root_pc = scale.root.to_semitone();
            let open_degrees: &[u8] = match scale.mode {
                Mode::Minor | Mode::Phrygian | Mode::Locrian | Mode::HarmonicMinor => {
                    // Minor-ish: 2nd (degree 2), 4th (5), b7 (10) in semitone offsets
                    &[2, 5, 10]
                }
                _ => &[2, 5, 11], // Major: 2nd (2), 4th (5), 7th (11)
            };
            let mut best: Option<u8> = None;
            let mut best_dist = i16::MAX;
            for midi in lo..=hi {
                let pc = midi % 12;
                let degree = (pc + 12 - root_pc) % 12;
                if !open_degrees.contains(&degree) {
                    continue;
                }
                if !mode_intervals.contains(&degree) {
                    continue;
                }
                let dist = (midi as i16 - prev_pitch as i16).abs();
                if dist > 12 {
                    continue; // stay within an octave of prev
                }
                if dist < best_dist {
                    best_dist = dist;
                    best = Some(midi);
                }
            }
            best
        }
    }
}

/// Pick a per-line phrase-start offset in beats, relative to the
/// rigid `syl_cursor * section_beats / total_syl` slot. Returns a
/// value that can be added to `line_start_beat_f` to break the
/// "every line starts on the downbeat" pattern.
///
/// Distribution (chosen to feel like written songs without sounding
/// random): 50 % downbeat (no offset), 25 % pickup (~half a bar
/// early — line starts late in the previous chord), 15 % off-beat
/// shift (+0.25 to +0.5 beats — syncopated start), 10 % anacrusis
/// (one whole beat early). Anchored by the seed so the same lyric
/// always lands on the same shape.
///
/// `line_idx` is included in the rng draw so each line picks
/// independently.
fn phrase_start_offset(rng: &mut XorShift, beats_per_bar: u32) -> f32 {
    let bpb = beats_per_bar.max(1) as f32;
    let r = rng.next_f32();
    if r < 0.50 {
        0.0
    } else if r < 0.75 {
        // Pickup: ~half a bar early.
        -bpb * 0.5
    } else if r < 0.90 {
        // Off-beat / syncopated start: 0.25 or 0.5 beats in.
        if rng.next_f32() < 0.5 {
            0.25
        } else {
            0.5
        }
    } else {
        // Anacrusis: one whole beat early.
        -1.0
    }
}

/// Combined velocity formula: base + phrase-arch contribution +
/// beat-of-bar accent + per-syllable jitter, clamped to [0.4, 1.0].
/// `arch_amount` controls the phrase-shape contribution (0 = flat,
/// 1 = full ±0.18 envelope swing); `accent_amount` weights the beat
/// strength contribution; `jitter` is the per-syllable random
/// half-width.
#[allow(clippy::too_many_arguments)]
fn shape_velocity(
    rng: &mut XorShift,
    base: f32,
    progress_in_line: f32,
    arch_amount: f32,
    beat: u32,
    beats_per_bar: u32,
    accent_amount: f32,
    jitter: f32,
) -> f32 {
    let arch = phrase_arch(progress_in_line) - 0.5; // -0.5..+0.5
    let accent = beat_strength(beat, beats_per_bar) - 0.5; // -0.5..+0.5
    let noise = (rng.next_f32() - 0.5) * 2.0 * jitter;
    (base + arch_amount * 0.36 * arch + accent_amount * 0.20 * accent + noise).clamp(0.4, 1.0)
}

/// Subtle per-syllable timing wobble — micro-rubato, ±`max_beats`
/// around the rigid grid position. Returns a beats-offset (positive
/// = ahead, negative = lag). Real singers don't sit exactly on the
/// click; tiny variation kills the "sequenced" feel.
fn rubato_offset(rng: &mut XorShift, max_beats: f32) -> f32 {
    (rng.next_f32() - 0.5) * 2.0 * max_beats
}

// ===========================================================================
// Style-profile trait + shared walker
// ===========================================================================
//
// Every per-style vocal generator (`derive_pop_ballad`, `derive_folk`, …)
// used to be a free function with a near-identical outer skeleton:
// destructure `VocalContext`, loop lines, loop syllables, push notes.
// The musical decisions inside that skeleton — pitch picking, slot
// width, rubato, duration, velocity, cadence — were the only parts
// that actually varied between styles. The trait below carves out
// exactly those decision points so the loop itself only has to be
// written once, in `walk_with_profile`.
//
// Profile authors do **not** override the walker structure. They
// implement methods on a unit struct (or, for Folk, a struct that
// carries the cross-line echo memory) and the walker calls them in
// the same order every style used in the legacy code. Preserving
// that order matters: the rng draws inside `pick_pitch` /
// `dur_beats` / `velocity` must happen in the same sequence the
// pre-refactor function used or the deterministic output drifts.

/// Per-line shared state computed once by the walker, then passed
/// (immutably) into every profile method for the line's syllables.
///
/// `extras` is the profile's private blob — Folk stores its echo
/// source and long/short ratios there, Anthemic its climax index,
/// Chant its triplet-feel flag, etc. The walker treats it as opaque.
struct LineState<E> {
    line_idx: usize,
    line_syl: u32,
    /// Prefix-sum of `line_syllables[0..line_idx]` — used by
    /// PopBallad to compute the contour curve's *global* (across-
    /// section) progress for the syllable, which differs from the
    /// per-line progress every other style uses.
    syl_cursor: u32,
    /// `(line_end_beat_f - line_start_beat_f) * (1 - breath_frac)` —
    /// Folk and Chant size their per-syllable slots from this.
    sing_span: f32,
    /// `sing_span / line_syl`. Profiles that don't reshape rhythm
    /// (PopBallad, Conversational, Hymnal, Anthemic) use this as
    /// every syllable's slot directly. Folk + Chant override `slot`
    /// to return per-syllable values that fluctuate around this.
    beat_step: f32,
    /// Prev-pitch *at the start of this line*. Folk reads this so
    /// echo lines can build their phrase offsets relative to the
    /// first pitch of the line they're echoing.
    line_first_pitch: u8,
    /// Effective register for this line. Hymnal narrows to a 9-st
    /// band on top of `ctx.lo`; Chant narrows around a centre. Others
    /// use `(ctx.lo, ctx.hi)`.
    band_lo: u8,
    band_hi: u8,
    articulation: f32,
    extras: E,
}

/// Per-syllable computed inputs the walker hands to the profile.
/// Built once per `s` and shared between `pick_pitch`, `dur_beats`,
/// and `velocity` so each method sees a consistent view of the
/// syllable's grid position + active chord.
struct StepInputs<'a> {
    s: u32,
    is_final: bool,
    progress_in_line: f32,
    beat_round: u32,
    chord: Option<&'a TimedChord>,
    prev_pitch: u8,
    /// This syllable's slot width in beats (= `line.beat_step` for
    /// most styles; overridden by Folk's long-short ratio and Chant's
    /// triplet-feel).
    slot: f32,
}

/// The contract each `VocalStyle` variant implements. The methods
/// expose every decision point that varied across the six legacy
/// per-style functions; everything else (line span math, beat-of-bar
/// chord lookup, `cap_interval`, the `enforce_no_overlap` pass) is
/// shared in `walk_with_profile`.
///
/// **rng-draw order:** the walker calls `slot`, then `rubato_max`,
/// then (if non-zero) draws one `rubato_offset`, then `pick_pitch`,
/// `finalize_pitch`, `dur_beats`, `velocity`. Implementors must
/// preserve their original draw order *within* each method —
/// reordering even a single `rng.next_f32()` changes the
/// deterministic output for that style.
trait VocalStyleProfile {
    /// Style-private per-line scratchpad. Set in [`begin_line`],
    /// read in the rest of the methods.
    type LineExtras: Default;

    /// Effective MIDI range for the style. Most styles use the
    /// user-set `(ctx.lo, ctx.hi)`; Hymnal caps to a 9-semitone band
    /// at the bottom, Chant narrows to a ~5-semitone band around the
    /// speaking pitch.
    fn band(&self, ctx: &VocalContext) -> (u8, u8) {
        (ctx.lo, ctx.hi)
    }

    /// Pitch the walker uses as the "previous pitch" for syllable 0
    /// of the first line. Each style anchors at a different register:
    /// PopBallad / Anthemic mid, Conversational a hair below mid,
    /// Folk near the top (descending phrases), Chant / Hymnal at
    /// the band centre.
    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8;

    /// Breath-gap fraction of each line that's silent. Some styles
    /// clamp the user-set `params.breath` (Folk ≥0.20, Chant ≥0.18,
    /// Anthemic ×0.6); Hymnal returns 0 (no breath, strict timing).
    fn breath_frac(&self, params: &VocalParams) -> f32 {
        params.breath.clamp(0.0, 0.9)
    }

    /// Whether to apply the random per-line phrase-start offset
    /// (pickup / anacrusis / off-beat). Hymnal returns `false` —
    /// strict timing is core to the style.
    fn use_phrase_start_offset(&self) -> bool {
        true
    }

    /// Build the per-line scratchpad. Called once before the
    /// syllable loop, after `LineState`'s shared fields are filled
    /// in. Free to consume rng draws — Folk uses one for its
    /// long-short ratio jitter, Chant one for its triplet-feel
    /// coin flip.
    fn begin_line(
        &mut self,
        rng: &mut XorShift,
        ctx: &VocalContext,
        line: &LineState<Self::LineExtras>,
    ) -> Self::LineExtras;

    /// Width of syllable `s`'s grid slot in beats. Default = the
    /// line's uniform `beat_step`. Folk overrides this for its
    /// long-short pairs; Chant overrides it for triplet-feel.
    fn slot(&self, line: &LineState<Self::LineExtras>, s: u32) -> f32 {
        let _ = s;
        line.beat_step
    }

    /// Maximum rubato half-width in beats for syllable `s`. The
    /// walker draws a rubato offset in `[-max, +max]` only when this
    /// is non-zero **and** `s` is neither the first nor the last
    /// syllable of the line (line edges stay on the grid so phrases
    /// still resolve to the chord). Default = 0 (strict grid).
    fn rubato_max(
        &self,
        line: &LineState<Self::LineExtras>,
        s: u32,
        slot: f32,
    ) -> f32 {
        let _ = (line, s, slot);
        0.0
    }

    /// Pick the syllable's pitch — handles cadence override on the
    /// final syllable, chord-tone anchoring on strong beats, and any
    /// style-specific snapping (e.g. `snap_to_pentatonic` for Folk).
    /// The walker then applies `cap_interval` against the line's
    /// effective band so the final pitch is at most `MAX_INTERVAL`
    /// semitones from the previous one.
    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<Self::LineExtras>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8;

    /// Syllable duration in beats. Profiles return whatever
    /// combination of `slot`, `beat_step`, articulation trim, and
    /// `rhythm_trim` rng jitter they used in the legacy code.
    fn dur_beats(
        &self,
        line: &LineState<Self::LineExtras>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32;

    /// Minimum duration in ticks (post-multiplication by `tpb`).
    /// Chant overrides this to `tpb / 8` so its tight sixteenth-feel
    /// notes aren't forced up to a quarter-note floor.
    fn min_dur_ticks(&self, tpb: u64) -> u64 {
        (tpb / 4).max(1)
    }

    /// MIDI velocity for the syllable. Always called — profiles
    /// build it from [`shape_velocity`] plus optional bumps
    /// (Anthemic's climax punch, Chant's spit lift).
    fn velocity(
        &self,
        line: &LineState<Self::LineExtras>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32;

    /// Called once per line after the syllable loop. Folk uses this
    /// to stash the line's pitch offsets in its echo memory so the
    /// next pair of lines can echo this contour. Default does
    /// nothing.
    fn end_line(&mut self, line_idx: usize, line_offsets: Vec<i16>) {
        let _ = (line_idx, line_offsets);
    }
}

/// The single per-syllable walker every `VocalStyleProfile`
/// implementation flows through. Replaces the six legacy
/// `derive_*` functions — each style is now just a profile.
fn walk_with_profile<P: VocalStyleProfile>(
    ctx: &VocalContext<'_>,
    mut profile: P,
) -> Vec<GeneratedNote> {
    let VocalContext {
        chords,
        params,
        tpb,
        section_beats,
        beats_per_bar,
        ref line_syllables,
        total_syl,
        seed,
        ..
    } = *ctx;

    let mut rng = XorShift::new(seed.max(1));
    let mut out = Vec::with_capacity(total_syl as usize);

    let band = profile.band(ctx);
    let mut prev_pitch = profile.init_prev_pitch(ctx, band);

    let breath_frac = profile.breath_frac(params);
    let articulation = params.articulation.clamp(0.0, 1.0);
    let min_dur = profile.min_dur_ticks(tpb);

    let mut syl_cursor: u32 = 0;
    for (line_idx, &line_syl) in line_syllables.iter().enumerate() {
        if line_syl == 0 {
            continue;
        }
        let raw_line_start =
            syl_cursor as f32 * section_beats as f32 / total_syl as f32;
        let line_offset = if profile.use_phrase_start_offset() {
            phrase_start_offset(&mut rng, beats_per_bar)
        } else {
            0.0
        };
        let line_start_beat_f = (raw_line_start + line_offset).max(0.0);
        let line_end_beat_f =
            (syl_cursor + line_syl) as f32 * section_beats as f32 / total_syl as f32;
        let line_beat_span = (line_end_beat_f - line_start_beat_f).max(0.001);
        let sing_span = line_beat_span * (1.0 - breath_frac);
        let beat_step = sing_span / line_syl as f32;
        let line_first_pitch = prev_pitch;
        let mut line = LineState {
            line_idx,
            line_syl,
            syl_cursor,
            sing_span,
            beat_step,
            line_first_pitch,
            band_lo: band.0,
            band_hi: band.1,
            articulation,
            extras: <P::LineExtras as Default>::default(),
        };
        line.extras = profile.begin_line(&mut rng, ctx, &line);

        let mut line_offsets: Vec<i16> = Vec::with_capacity(line_syl as usize);
        let mut beat_cursor = 0.0_f32;
        for s in 0..line_syl {
            let progress_in_line = s as f32 / line_syl.max(1) as f32;
            let slot = profile.slot(&line, s);
            let rubato_max = profile.rubato_max(&line, s, slot);
            let rubato = if s == 0 || s + 1 == line_syl || rubato_max == 0.0 {
                0.0
            } else {
                rubato_offset(&mut rng, rubato_max)
            };
            let beat_f = line_start_beat_f + beat_cursor + rubato;
            beat_cursor += slot;
            let beat_round = beat_f.floor().clamp(0.0, (section_beats - 1) as f32) as u32;
            let chord = chord_at_beat(chords, beat_round);
            let is_final = s + 1 == line_syl;
            let inp = StepInputs {
                s,
                is_final,
                progress_in_line,
                beat_round,
                chord,
                prev_pitch,
                slot,
            };
            let raw = profile.pick_pitch(ctx, &line, &inp, &mut rng);
            let pitch = cap_interval(prev_pitch, raw, line.band_lo, line.band_hi, ctx.scale);
            let dur_beats = profile.dur_beats(&line, &inp, &mut rng, beats_per_bar);
            let velocity = profile.velocity(&line, &inp, &mut rng, beats_per_bar);
            let start_tick = (beat_f as f64 * tpb as f64) as u64;
            let dur_ticks = ((dur_beats as f64 * tpb as f64) as u64).max(min_dur);
            out.push(GeneratedNote {
                note: pitch,
                velocity,
                start_tick,
                duration_ticks: dur_ticks,
            });
            line_offsets.push(pitch as i16 - line_first_pitch as i16);
            prev_pitch = pitch;
        }
        profile.end_line(line_idx, line_offsets);
        syl_cursor += line_syl;
    }
    out
}

/// Dispatch table — picks the concrete profile for a `VocalStyle`
/// variant and runs the shared walker.
fn derive_with_profile(ctx: &VocalContext<'_>) -> Vec<GeneratedNote> {
    match ctx.params.style {
        VocalStyle::PopBallad => walk_with_profile(ctx, PopBalladProfile),
        VocalStyle::Conversational => walk_with_profile(ctx, ConversationalProfile),
        VocalStyle::Hymnal => walk_with_profile(ctx, HymnalProfile),
        VocalStyle::Folk => walk_with_profile(ctx, FolkProfile::default()),
        VocalStyle::Anthemic => walk_with_profile(ctx, AnthemicProfile),
        VocalStyle::Chant => walk_with_profile(ctx, ChantProfile),
    }
}


// ===========================================================================
// Per-style profile implementations
// ===========================================================================
//
// Each profile is a unit struct (Folk carries echo state) whose
// `impl VocalStyleProfile` mirrors the legacy free function for that
// style. The numeric constants and the order of rng draws match the
// pre-refactor code one-for-one so the deterministic output is byte-
// identical to what the old `derive_pop_ballad` / `derive_folk` / …
// emitted.

/// PopBallad: stepwise contour-driven walk with breath gaps, gentle
/// chord-tone anchoring on strong beats, surprise passing-leap on a
/// minority of weak-beat syllables, antecedent/consequent cadence on
/// each line's terminal note. The legacy default — the only style
/// that fully respects every Melody slider (contour, anchor, leap).
struct PopBalladProfile;

impl VocalStyleProfile for PopBalladProfile {
    type LineExtras = ();

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        snap_to_scale(((band.0 as u16 + band.1 as u16) / 2) as u8, ctx.scale, band.0, band.1)
    }

    fn begin_line(
        &mut self,
        _rng: &mut XorShift,
        _ctx: &VocalContext,
        _line: &LineState<()>,
    ) {
    }

    fn rubato_max(&self, line: &LineState<()>, _s: u32, _slot: f32) -> f32 {
        line.beat_step * 0.05
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        // Strong-beat heuristic + anchor draw — match original order.
        let strong = inp.s == 0
            || inp.s + 1 == line.line_syl
            || (inp.s.is_multiple_of(2) && beat_strength(inp.beat_round, ctx.beats_per_bar) > 0.5);
        let anchor = strong && rng.next_f32() < ctx.params.chord_tone_anchor;

        // Contour target uses GLOBAL progress across the section,
        // not per-line — Arch arches over the whole section.
        let global_idx = line.syl_cursor + inp.s;
        let global_t =
            global_idx as f32 / (ctx.total_syl.saturating_sub(1).max(1)) as f32;
        let contour_pos = contour_height(ctx.params.contour, global_t).clamp(0.0, 1.0);
        let contour_target = line.band_lo as f32
            + contour_pos * (line.band_hi as f32 - line.band_lo as f32);
        let pulled = inp.prev_pitch as f32 * 2.0 / 3.0 + contour_target / 3.0;

        // Step vs leap draws — preserve original order.
        let leap = rng.next_f32() < ctx.params.leap_range;
        let surprise_leap = !leap && rng.next_f32() < 0.12;
        let step_range = if leap {
            3..=6
        } else if surprise_leap {
            3..=4
        } else {
            1..=2
        };
        let step = (rng.next_range(*step_range.end() - *step_range.start() + 1)
            + *step_range.start()) as i16;
        let direction = if contour_target > inp.prev_pitch as f32 { 1i16 } else { -1 };
        let walked =
            (pulled as i16 + step * direction).clamp(line.band_lo as i16, line.band_hi as i16) as u8;

        if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                if anchor {
                    inp.chord
                        .and_then(|c| {
                            chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inp.prev_pitch)
                        })
                        .unwrap_or(walked)
                } else if ctx.params.stay_in_scale {
                    snap_to_scale(walked, ctx.scale, line.band_lo, line.band_hi)
                } else {
                    walked
                }
            })
        } else if anchor {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inp.prev_pitch))
                .unwrap_or(walked)
        } else if ctx.params.stay_in_scale {
            snap_to_scale(walked, ctx.scale, line.band_lo, line.band_hi)
        } else {
            walked
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let base_trim = 0.98 - 0.48 * line.articulation;
        if inp.is_final {
            terminal_dur_beats(line.beat_step, line.articulation)
        } else {
            let trim = rhythm_trim(rng, base_trim, inp.beat_round, beats_per_bar, 0.18);
            line.beat_step * trim
        }
    }

    fn velocity(
        &self,
        _line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        shape_velocity(
            rng,
            0.74,
            inp.progress_in_line,
            0.9,
            inp.beat_round,
            beats_per_bar,
            0.7,
            0.08,
        )
    }
}

/// Conversational: talky/spoken-feel anchored on a speaking pitch a
/// hair below the centre of the range. Pitches repeat ~55 % of the
/// time, walk by 1–2 semitones otherwise, with line-edge inflection
/// (rise on syllable 0, fall on the terminal). Larger rubato than
/// PopBallad — words push and pull against the click. Ignores
/// `params.contour` because the shape comes from the per-line
/// inflection, not a section-spanning curve.
struct ConversationalProfile;

/// Speaking pitch helper: a hair below the band centre, scale-snapped.
fn conversational_speaking_pitch(ctx: &VocalContext, band: (u8, u8)) -> u8 {
    let span = band.1 as i16 - band.0 as i16;
    snap_to_scale(
        (band.0 as i16 + (span * 4) / 10).clamp(band.0 as i16, band.1 as i16) as u8,
        ctx.scale,
        band.0,
        band.1,
    )
}

impl VocalStyleProfile for ConversationalProfile {
    type LineExtras = ();

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        conversational_speaking_pitch(ctx, band)
    }

    fn begin_line(&mut self, _rng: &mut XorShift, _ctx: &VocalContext, _line: &LineState<()>) {}

    fn rubato_max(&self, line: &LineState<()>, _s: u32, _slot: f32) -> f32 {
        line.beat_step * 0.10
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        // Pitch repetition with bursts. The two repeat branches look
        // collapsible but they're *not*: each performs an independent
        // rng draw and the second only runs when the first failed.
        // Merging them would advance rng one fewer step per syllable
        // and the deterministic output for Conversational would
        // drift. The first branch (~10 %) flags a "spoken-emphasis
        // run" where the next syllable also re-uses prev_pitch; the
        // second branch (~55 % of the remaining 90 %) is the regular
        // repeat. Order of draws matches `derive_conversational`.
        #[allow(clippy::if_same_then_else)]
        let pitch_pre = if rng.next_f32() < 0.10 {
            inp.prev_pitch
        } else if rng.next_f32() < 0.55 {
            inp.prev_pitch
        } else {
            let dir: i16 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let step: i16 = if rng.next_f32() < 0.18 { 2 } else { 1 };
            ((inp.prev_pitch as i16 + dir * step).clamp(line.band_lo as i16, line.band_hi as i16))
                as u8
        };

        let speaking = conversational_speaking_pitch(ctx, (line.band_lo, line.band_hi));
        let inflected = if inp.s == 0 {
            ((speaking as i16 + 1).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else if inp.s + 1 == line.line_syl {
            ((speaking as i16 - 1).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else {
            pitch_pre
        };

        let strong = inp.s == 0
            || inp.s + 1 == line.line_syl
            || beat_strength(inp.beat_round, ctx.beats_per_bar) >= 0.65;
        if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                inp.chord
                    .and_then(|c| {
                        chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inflected)
                    })
                    .unwrap_or(inflected)
            })
        } else if strong {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inflected))
                .unwrap_or(inflected)
        } else if ctx.params.stay_in_scale {
            snap_to_scale(inflected, ctx.scale, line.band_lo, line.band_hi)
        } else {
            inflected
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.95 - 0.45 * line.articulation;
        if inp.is_final {
            terminal_dur_beats(line.beat_step, line.articulation)
        } else {
            let trim_local = rhythm_trim(rng, trim, inp.beat_round, beats_per_bar, 0.22);
            line.beat_step * trim_local
        }
    }

    fn velocity(
        &self,
        _line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        shape_velocity(
            rng,
            0.62,
            inp.progress_in_line,
            0.6,
            inp.beat_round,
            beats_per_bar,
            0.5,
            0.06,
        )
    }
}

/// Hymnal: strict syllable-per-quarter rhythm with stepwise motion
/// only, narrowed to a 9-semitone band on top of `ctx.lo`, every
/// line cadencing on a chord tone of the active chord. Minimal
/// randomness — same seed gives a near-deterministic shape. Skips
/// the random phrase-start offset (strict timing is core to the
/// style) and skips the breath-gap fraction (every syllable
/// occupies its full grid slot).
struct HymnalProfile;

impl VocalStyleProfile for HymnalProfile {
    type LineExtras = ();

    fn band(&self, ctx: &VocalContext) -> (u8, u8) {
        let band_hi = (ctx.lo as i16 + 9).min(ctx.hi as i16) as u8;
        (ctx.lo, band_hi)
    }

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        snap_to_scale(((band.0 as u16 + band.1 as u16) / 2) as u8, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, _params: &VocalParams) -> f32 {
        0.0
    }

    fn use_phrase_start_offset(&self) -> bool {
        false
    }

    fn begin_line(&mut self, _rng: &mut XorShift, _ctx: &VocalContext, _line: &LineState<()>) {}

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        let centre = (line.band_lo as i16 + line.band_hi as i16) / 2;
        let drift = (centre - inp.prev_pitch as i16).signum();
        // Stepwise walk with centre-bias — order of draws matches
        // the original. Note the nested `else if rng.next_f32()` is
        // *not* short-circuited by an earlier success: each branch
        // is reached only if its predecessor failed.
        let raw_step: i16 = if rng.next_f32() < 0.12 {
            0
        } else if rng.next_f32() < 0.6 {
            drift
        } else if rng.next_f32() < 0.5 {
            1
        } else {
            -1
        };
        let mut candidate = ((inp.prev_pitch as i16 + raw_step)
            .clamp(line.band_lo as i16, line.band_hi as i16)) as u8;

        if inp.is_final {
            if let Some(picked) = cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            ) {
                candidate = picked;
            } else if let Some(c) = inp.chord {
                let tones = chord_tones_in_register(c.chord, (line.band_lo, line.band_hi));
                if let Some(picked) = tones
                    .iter()
                    .copied()
                    .min_by_key(|t| (*t as i16 - inp.prev_pitch as i16).abs())
                {
                    candidate = picked;
                }
            }
        }

        snap_to_scale(candidate, ctx.scale, line.band_lo, line.band_hi)
    }

    fn dur_beats(
        &self,
        line: &LineState<()>,
        _inp: &StepInputs<'_>,
        _rng: &mut XorShift,
        _beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.92 - 0.30 * line.articulation;
        line.beat_step * trim
    }

    fn velocity(
        &self,
        _line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        shape_velocity(
            rng,
            0.72,
            inp.progress_in_line,
            0.45,
            inp.beat_round,
            beats_per_bar,
            0.4,
            0.05,
        )
    }
}

/// Folk: pentatonic, descending-leaning phrases with long-short
/// rhythm pairs (odd syllables stretched, even compressed). Lines 2
/// and 3 echo the pitch contour of lines 0 and 1 — the call-and-
/// response shape characteristic of folk songs. Lines 0 and 1 store
/// their pitch-offset memory in `echo_offsets`; lines 2+ read it
/// back through the line index mod 2.
#[derive(Default)]
struct FolkProfile {
    /// Per-line pitch offsets (signed semitones relative to
    /// `line_first_pitch`) for line 0 and line 1. Populated by
    /// `end_line` so the second pair of lines can echo them.
    echo_offsets: [Vec<i16>; 2],
}

#[derive(Default, Clone)]
struct FolkLine {
    pair_unit: f32,
    long_ratio: f32,
    short_ratio: f32,
    start_pitch: u8,
    descend_span: f32,
    /// Captured at `begin_line` so `pick_pitch` can read it without
    /// borrowing `&mut self`.
    echo_source: Option<Vec<i16>>,
}

impl VocalStyleProfile for FolkProfile {
    type LineExtras = FolkLine;

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        let span = band.1 as i16 - band.0 as i16;
        let start_pitch =
            (band.0 as i16 + (span * 3) / 4).clamp(band.0 as i16, band.1 as i16) as u8;
        snap_to_pentatonic(start_pitch, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, params: &VocalParams) -> f32 {
        params.breath.clamp(0.0, 0.9).max(0.20)
    }

    fn begin_line(
        &mut self,
        rng: &mut XorShift,
        _ctx: &VocalContext,
        line: &LineState<FolkLine>,
    ) -> FolkLine {
        // Per-line long-short ratio jitter — one rng draw, must
        // happen before any syllable draws to match the original
        // `derive_folk` sequence.
        let long_ratio = 1.35 + (rng.next_f32() - 0.5) * 0.40;
        let short_ratio = 2.0 - long_ratio;
        let pair_unit = line.sing_span / line.line_syl as f32;

        // Echo source is line_idx >= 2's line (line_idx % 2). The
        // legacy code used `.filter(|v| !v.is_empty())` to gate the
        // echo when the previous line was empty.
        let echo_source: Option<Vec<i16>> = if line.line_idx >= 2 {
            self.echo_offsets
                .get(line.line_idx % 2)
                .filter(|v| !v.is_empty())
                .cloned()
        } else {
            None
        };

        let span = line.band_hi as i16 - line.band_lo as i16;
        let start_pitch = (line.band_lo as i16 + (span * 3) / 4)
            .clamp(line.band_lo as i16, line.band_hi as i16) as u8;
        FolkLine {
            pair_unit,
            long_ratio,
            short_ratio,
            start_pitch,
            descend_span: span as f32 * 0.45,
            echo_source,
        }
    }

    fn slot(&self, line: &LineState<FolkLine>, s: u32) -> f32 {
        if s.is_multiple_of(2) {
            line.extras.pair_unit * line.extras.long_ratio
        } else {
            line.extras.pair_unit * line.extras.short_ratio
        }
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<FolkLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        let candidate = if let Some(source) = line.extras.echo_source.as_ref() {
            let mut off = source.get(inp.s as usize).copied().unwrap_or(0);
            if rng.next_f32() < 0.25 {
                off += if rng.next_f32() < 0.5 { 1 } else { -1 };
            }
            ((line.line_first_pitch as i16 + off)
                .clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else {
            let descend_target = (line.extras.start_pitch as f32
                - inp.progress_in_line * line.extras.descend_span)
                .clamp(line.band_lo as f32, line.band_hi as f32)
                as u8;
            // Order of draws matches `derive_folk`: 0.05 first, then
            // 0.35; the inner `0.5` only fires if 0.35 succeeded.
            let jitter = if rng.next_f32() < 0.05 {
                -((rng.next_range(3) as i16) + 3) // -3..-5
            } else if rng.next_f32() < 0.35 {
                if rng.next_f32() < 0.5 { 1 } else { -1 }
            } else {
                0
            };
            ((descend_target as i16 + jitter).clamp(line.band_lo as i16, line.band_hi as i16))
                as u8
        };

        let is_long = inp.s.is_multiple_of(2);
        let strong = inp.s == 0
            || inp.s + 1 == line.line_syl
            || (is_long && beat_strength(inp.beat_round, ctx.beats_per_bar) >= 0.65);
        if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                inp.chord
                    .and_then(|c| {
                        chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate)
                    })
                    .unwrap_or(candidate)
            })
        } else if strong {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate))
                .unwrap_or(candidate)
        } else {
            snap_to_pentatonic(candidate, ctx.scale, line.band_lo, line.band_hi)
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<FolkLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.92 - 0.40 * line.articulation;
        if inp.is_final {
            terminal_dur_beats(line.extras.pair_unit, line.articulation)
        } else {
            let trim_local = rhythm_trim(rng, trim, inp.beat_round, beats_per_bar, 0.12);
            inp.slot * trim_local
        }
    }

    fn velocity(
        &self,
        _line: &LineState<FolkLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        shape_velocity(
            rng,
            0.70,
            inp.progress_in_line,
            0.7,
            inp.beat_round,
            beats_per_bar,
            0.85,
            0.10,
        )
    }

    fn end_line(&mut self, line_idx: usize, line_offsets: Vec<i16>) {
        if line_idx < 2 {
            self.echo_offsets[line_idx] = line_offsets;
        }
    }
}

/// Anthemic: wide-range chorus melody. Each line arcs to a peak
/// roughly 60 % through the line and the syllable closest to that
/// peak (the "money note") leaps to the highest in-range chord tone.
/// Final syllables get an extra-long sustain (1.6× beat-step vs
/// 1.4× elsewhere) for the held-cadence chorus feel. Strict grid
/// (no rubato), with strong-beat chord-tone anchoring on every
/// other syllable.
struct AnthemicProfile;

#[derive(Default, Clone)]
struct AnthemicLine {
    /// Index of the climax syllable (= round(line_syl * 0.6)), or
    /// `u32::MAX` when `line_syl < 4` (too short for a climax).
    climax_idx: u32,
    has_climax: bool,
}

impl VocalStyleProfile for AnthemicProfile {
    type LineExtras = AnthemicLine;

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        snap_to_scale(((band.0 as u16 + band.1 as u16) / 2) as u8, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, params: &VocalParams) -> f32 {
        (params.breath.clamp(0.0, 0.9) * 0.6).max(0.10)
    }

    fn begin_line(
        &mut self,
        _rng: &mut XorShift,
        _ctx: &VocalContext,
        line: &LineState<AnthemicLine>,
    ) -> AnthemicLine {
        let climax_idx = ((line.line_syl as f32 * 0.6).round() as u32).min(line.line_syl - 1);
        AnthemicLine {
            climax_idx,
            has_climax: line.line_syl >= 4,
        }
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<AnthemicLine>,
        inp: &StepInputs<'_>,
        _rng: &mut XorShift,
    ) -> u8 {
        // Per-line arch: peak at t=0.6. No rng draws here.
        let t = inp.progress_in_line;
        let arch = 1.0 - ((t - 0.6).abs() / 0.6_f32.max(1.0 - 0.6_f32)).clamp(0.0, 1.0);
        let span = line.band_hi as f32 - line.band_lo as f32;
        let target = line.band_lo as f32 + (0.30 + 0.60 * arch) * span;

        let strong = inp.s.is_multiple_of(2);
        let is_climax = inp.s == line.extras.climax_idx && line.extras.has_climax;
        let candidate = target.clamp(line.band_lo as f32, line.band_hi as f32) as u8;

        if is_climax {
            inp.chord
                .and_then(|c| {
                    let tones = chord_tones_in_register(c.chord, (line.band_lo, line.band_hi));
                    tones.into_iter().max()
                })
                .unwrap_or(candidate)
        } else if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                inp.chord
                    .and_then(|c| {
                        chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate)
                    })
                    .unwrap_or(candidate)
            })
        } else if strong {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate))
                .unwrap_or(candidate)
        } else if ctx.params.stay_in_scale {
            snap_to_scale(candidate, ctx.scale, line.band_lo, line.band_hi)
        } else {
            candidate
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<AnthemicLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.95 - 0.30 * line.articulation;
        let is_climax = inp.s == line.extras.climax_idx && line.extras.has_climax;
        if inp.is_final {
            // Anthemic terminal: 1.6× beat-step floor (vs the
            // shared 1.4× cap in `terminal_dur_beats`), so the
            // chorus money-note holds longer than other styles'.
            let cap = line.beat_step * 1.6;
            let normal = line.beat_step * trim;
            cap.max(normal)
        } else if is_climax {
            line.beat_step * 1.4
        } else {
            let trim_local = rhythm_trim(rng, trim, inp.beat_round, beats_per_bar, 0.20);
            line.beat_step * trim_local
        }
    }

    fn velocity(
        &self,
        line: &LineState<AnthemicLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let mut v = shape_velocity(
            rng,
            0.80,
            inp.progress_in_line,
            1.0,
            inp.beat_round,
            beats_per_bar,
            0.8,
            0.07,
        );
        let is_climax = inp.s == line.extras.climax_idx && line.extras.has_climax;
        if is_climax {
            v = (v + 0.10).clamp(0.4, 1.0);
        }
        v
    }
}

/// Chant: hip-hop / spoken-word monotone-leaning vocal anchored on
/// the chord root in a narrow 5-semitone band around the speaking
/// pitch. Bursts of fast syllables (sixteenth-feel by default,
/// triplet-feel on ~30 % of lines) pack into the front of each line
/// with a wider breath-gap at the end. Occasional "spit" — a 3-4
/// semitone lift on a non-edge syllable for emphasis, then snap back.
struct ChantProfile;

#[derive(Default, Clone)]
struct ChantLine {
    /// Speaking-pitch centre, computed from `ctx.lo` / `ctx.hi`
    /// (NOT the narrowed band — match the legacy `derive_chant`
    /// math exactly or seed-anchored output drifts).
    band_centre: u8,
    /// True ~30 % of lines — switches the syllable slot from
    /// straight sixteenths to a triplet-swing pattern.
    triplet_feel: bool,
    /// Pre-triplet-jitter slot width = `sing_span / line_syl`.
    base_slot: f32,
}

impl VocalStyleProfile for ChantProfile {
    type LineExtras = ChantLine;

    fn band(&self, ctx: &VocalContext) -> (u8, u8) {
        let span = ctx.hi as i16 - ctx.lo as i16;
        let centre = (ctx.lo as i16 + (span * 4) / 10).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        let band_lo = (centre as i16 - 2).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        let band_hi = (centre as i16 + 3).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        (band_lo, band_hi)
    }

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        // Original chant `centre` is built from `ctx.lo`/`ctx.hi`,
        // not from the band — match that or the deterministic
        // output drifts.
        let span = ctx.hi as i16 - ctx.lo as i16;
        let centre = (ctx.lo as i16 + (span * 4) / 10).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        snap_to_scale(centre, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, params: &VocalParams) -> f32 {
        params.breath.clamp(0.0, 0.9).max(0.18)
    }

    fn min_dur_ticks(&self, tpb: u64) -> u64 {
        (tpb / 8).max(1)
    }

    fn begin_line(
        &mut self,
        rng: &mut XorShift,
        ctx: &VocalContext,
        line: &LineState<ChantLine>,
    ) -> ChantLine {
        // Triplet-feel draw is the first rng action of the line —
        // matches original `derive_chant`.
        let triplet_feel = rng.next_f32() < 0.30;
        let base_slot = line.sing_span / line.line_syl as f32;
        // Centre uses ctx.lo / ctx.hi, not the narrowed band.
        let span = ctx.hi as i16 - ctx.lo as i16;
        let centre =
            (ctx.lo as i16 + (span * 4) / 10).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        ChantLine {
            band_centre: centre,
            triplet_feel,
            base_slot,
        }
    }

    fn slot(&self, line: &LineState<ChantLine>, s: u32) -> f32 {
        if line.extras.triplet_feel {
            match s % 3 {
                0 => line.extras.base_slot * 1.20,
                _ => line.extras.base_slot * 0.90,
            }
        } else {
            line.extras.base_slot
        }
    }

    fn rubato_max(&self, _line: &LineState<ChantLine>, _s: u32, slot: f32) -> f32 {
        slot * 0.08
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<ChantLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        // Order: punch_down draw → spit draw → spit-lift draws OR
        // the s%4 branch. Must match `derive_chant` exactly.
        let centre = line.extras.band_centre;
        let punch_down = beat_strength(inp.beat_round, ctx.beats_per_bar) >= 0.65
            && rng.next_f32() < 0.5;
        let spit = !punch_down
            && rng.next_f32() < 0.08
            && inp.s > 0
            && inp.s + 1 < line.line_syl;
        // Stash the spit flag for `velocity` to pick up so it can
        // apply the extra +0.12 lift without re-drawing the
        // predicate (which would advance rng out of sync).
        CHANT_SPIT_FLAG.with(|f| f.set(spit));
        let pitch_pre = if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                inp.chord
                    .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), centre))
                    .unwrap_or(centre)
            })
        } else if inp.s == 0 {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), centre))
                .unwrap_or(centre)
        } else if punch_down {
            inp.chord
                .and_then(|c| {
                    chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inp.prev_pitch)
                })
                .unwrap_or(inp.prev_pitch)
        } else if spit {
            let lift = (rng.next_range(2) as i16) + 3;
            ((inp.prev_pitch as i16 + lift).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else if inp.s.is_multiple_of(4) && rng.next_f32() < 0.55 {
            let dir: i16 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            ((inp.prev_pitch as i16 + dir).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else {
            inp.prev_pitch
        };

        snap_to_scale(pitch_pre, ctx.scale, line.band_lo, line.band_hi)
    }

    fn dur_beats(
        &self,
        _line: &LineState<ChantLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        inp.slot * rhythm_trim(rng, 0.85, inp.beat_round, beats_per_bar, 0.20)
    }

    fn velocity(
        &self,
        _line: &LineState<ChantLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        // Chant's velocity bump (the "spit" lift) shares the same
        // boolean `pick_pitch` already drew. Re-drawing it here would
        // desync rng, so `pick_pitch` stashes the flag in a
        // thread-local that we drain on read.
        let mut v = shape_velocity(
            rng,
            0.65,
            inp.progress_in_line,
            0.4,
            inp.beat_round,
            beats_per_bar,
            1.0,
            0.10,
        );
        if CHANT_SPIT_FLAG.with(|f| f.replace(false)) {
            v = (v + 0.12).clamp(0.4, 1.0);
        }
        v
    }
}

// Used to pass Chant's per-syllable `spit` flag from `pick_pitch` to
// `velocity` without reordering rng draws or adding an extra
// step-output field. Set in `pick_pitch`, read+cleared in `velocity`.
// Single-threaded use — the walker runs synchronously.
thread_local! {
    static CHANT_SPIT_FLAG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Adopt the chord root + quality of the first chord as a coarse scale
/// guess when the caller doesn't pass one explicitly. Used by
/// `derive_vocal` for its in-line snapping when `stay_in_scale` is set.
fn scale_from_chords(chords: &[TimedChord]) -> Option<Scale> {
    use crate::scale::Mode;
    chords.first().map(|c| {
        let mode = match c.chord.quality {
            crate::chord::ChordQuality::Min | crate::chord::ChordQuality::Min7 => Mode::Minor,
            _ => Mode::Major,
        };
        Scale::new(c.chord.root, mode)
    })
}
