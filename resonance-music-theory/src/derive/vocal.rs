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

    // ---- Voice & delivery ----
    pub timbre: VocalTimbre,
    pub vibrato: f32,
    pub articulation: f32,
    pub consonant_emphasis: f32,
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
            timbre: VocalTimbre::Warm,
            vibrato: 0.30,
            articulation: 0.65,
            consonant_emphasis: 0.40,
        }
    }
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
fn count_syllables(text: &str) -> u32 {
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

/// Derive MIDI notes for a vocal line. Walks the section once per
/// syllable: places each syllable on its slot, picks a chord-aware
/// pitch shaped by the contour, leap bias, and scale.
pub fn derive_vocal(
    chords: &[TimedChord],
    params: &VocalParams,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<GeneratedNote> {
    if chords.is_empty() || params.draft.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let section_beats = total_beats(chords);
    if section_beats == 0 {
        return Vec::new();
    }
    let (lo, hi) = params.range;
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    if lo == hi {
        return Vec::new();
    }

    // Total syllables across all draft lines.
    //
    // We use `count_syllables` (mechanical count of `·`-separators +
    // whitespace) rather than the corpus-author's `syllables` field
    // because the two sometimes disagree, and the SVS render path
    // walks words/syllables using the *same* mechanical count. Trusting
    // the corpus field here can leave dangling notes with no phoneme
    // chunk to fill them, which surface as phantom "ah" syllables in
    // the rendered audio (e.g. "re·mem·ber-AH").
    let total_syl: u32 = params
        .draft
        .iter()
        .map(|l| count_syllables(&l.text))
        .sum();
    if total_syl == 0 {
        return Vec::new();
    }

    let mut rng = XorShift::new(seed.max(1));
    let mut out = Vec::with_capacity(total_syl as usize);

    // Pre-compute the scale used for in-key snapping. Cheap enough to
    // hoist out of the per-syllable loop.
    let scale = if params.stay_in_scale {
        scale_from_chords(chords)
    } else {
        None
    };

    // Start near the middle of the range, snapped to scale.
    let mut prev_pitch = snap_to_scale(((lo as u16 + hi as u16) / 2) as u8, scale, lo, hi);

    // The breath gap between phrases — eats a fraction of each phrase's
    // tail. A phrase = one lyric line.
    let breath_frac = params.breath.clamp(0.0, 0.9);

    // Walk lines. Each line claims (line_syllables / total) of the
    // section. Beat positions are continuous across lines so the section
    // packs cleanly.
    let mut syl_cursor: u32 = 0;
    for line in &params.draft {
        let line_syl = count_syllables(&line.text);
        if line_syl == 0 {
            continue;
        }
        let line_start_beat_f = syl_cursor as f32 * section_beats as f32 / total_syl as f32;
        let line_end_beat_f = (syl_cursor + line_syl) as f32 * section_beats as f32 / total_syl as f32;
        let line_beat_span = (line_end_beat_f - line_start_beat_f).max(0.001);
        let sing_span = line_beat_span * (1.0 - breath_frac);
        let beat_step = sing_span / line_syl as f32;

        for s in 0..line_syl {
            let beat_f = line_start_beat_f + s as f32 * beat_step;
            let beat_round = beat_f.floor().clamp(0.0, (section_beats - 1) as f32) as u32;
            let chord = chord_at_beat(chords, beat_round);

            // Strong-beat heuristic: first syllable of a line and every
            // second syllable after that count as on-beat.
            let strong = s == 0 || s % 2 == 0;
            let anchor = strong && rng.next_f32() < params.chord_tone_anchor;

            // Contour target — global progress across the section, not
            // per-line, so an Arch shape arches over the whole section.
            let global_t = (syl_cursor + s) as f32 / (total_syl.saturating_sub(1).max(1)) as f32;
            let contour_pos = contour_height(params.contour, global_t).clamp(0.0, 1.0);
            let contour_target = lo as f32
                + contour_pos * (hi as f32 - lo as f32);
            // Pull toward contour by 1/3 the gap.
            let pulled = prev_pitch as f32 * 2.0 / 3.0 + contour_target / 3.0;

            // Step vs leap.
            let leap = rng.next_f32() < params.leap_range;
            // Smaller intervals than before — 5-8 semitone "leaps"
            // every other syllable produced octave-spanning vocal
            // lines that the SVS model can't render cleanly. Real
            // sung melodies stay mostly within a 3rd, with the
            // occasional 4th/5th. Keep "step" at minor seconds
            // (1-2 semitones) so neighbouring syllables don't lurch.
            let step_range = if leap { 3..=5 } else { 1..=2 };
            let step = (rng.next_range(*step_range.end() - *step_range.start() + 1)
                + *step_range.start()) as i16;
            let direction = if contour_target > prev_pitch as f32 { 1i16 } else { -1 };
            let walked = (pulled as i16 + step * direction).clamp(lo as i16, hi as i16) as u8;

            // Anchor to chord tone if requested. Picks the chord tone
            // nearest the *previous* pitch (not the walked target) so
            // anchoring never introduces a bigger jump than the step/
            // leap math allowed. Plus we cap the resulting interval at
            // a major-6th (9 semitones) from the previous pitch so the
            // melody stays singable. Falls back to `walked` when no
            // chord is available or no chord tones live in range.
            let raw_pitch = if anchor {
                chord
                    .map(|c| chord_tones_in_register(c.chord, (lo, hi)))
                    .and_then(|tones| {
                        tones
                            .iter()
                            .min_by_key(|t| (**t as i16 - prev_pitch as i16).abs())
                            .copied()
                    })
                    .unwrap_or(walked)
            } else if params.stay_in_scale {
                snap_to_scale(walked, scale, lo, hi)
            } else {
                walked
            };
            // Clamp the absolute interval from prev_pitch. If the
            // candidate exceeds the cap, step in that direction by the
            // cap amount and re-snap to scale so we stay musical.
            const MAX_INTERVAL: i16 = 9;
            let delta = raw_pitch as i16 - prev_pitch as i16;
            let pitch = if delta.abs() > MAX_INTERVAL {
                let dir = delta.signum();
                let capped =
                    (prev_pitch as i16 + dir * MAX_INTERVAL).clamp(lo as i16, hi as i16) as u8;
                if params.stay_in_scale {
                    snap_to_scale(capped, scale, lo, hi)
                } else {
                    capped
                }
            } else {
                raw_pitch
            };

            // Duration in ticks: one syllable's worth of the beat slot,
            // trimmed by the user's articulation slider. Low
            // articulation → nearly the full slot (legato, tiny gap);
            // high articulation → ~half the slot (staccato, audible
            // gap that the SVS pipeline turns into a breath / SP).
            let articulation = params.articulation.clamp(0.0, 1.0);
            let trim = 0.98 - 0.48 * articulation;
            let dur_beats = beat_step * trim;
            let start_tick = (beat_f as f64 * tpb as f64) as u64;
            let dur_ticks = ((dur_beats as f64 * tpb as f64) as u64).max(tpb / 4);

            let velocity_base = 0.78 + if strong { 0.08 } else { 0.0 };
            let velocity_jitter = (rng.next_f32() - 0.5) * 0.06;
            let velocity = (velocity_base + velocity_jitter).clamp(0.4, 1.0);

            out.push(GeneratedNote {
                note: pitch,
                velocity,
                start_tick,
                duration_ticks: dur_ticks,
            });
            prev_pitch = pitch;
        }

        syl_cursor += line_syl;
    }

    out
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
