//! What Resonance knows about specific banks that their files do not say
//! (vocal-mcp-control-and-quality.md §4.3, D10): display names, which of
//! a bank's speakers to offer and what to call them, comfortable ranges,
//! phoneme substitution hints, terms of use — and, for the banks
//! Resonance has shipped or read the files of (the three development
//! banks and the LUNAI English singers), the facts their configs declare,
//! so a lane on a bank that is not installed still validates and reports
//! the way it will once the bank is there.
//!
//! Derived facts win where both exist: an installed bank's curve flags,
//! phoneme inventory and language ids come from its files
//! ([`super::InstalledBank`]); this table only overrides names, the speaker
//! allow-list and the facts no file carries.

use resonance_music_theory::VoiceType;
use resonance_svs::voicebank::CurveSupport;

/// One speaker (voice mode) the built-in table offers for a bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinSpeaker {
    /// The wire / project id (`disco`).
    pub id: &'static str,
    /// What the inspector shows (`Disco`).
    pub name: &'static str,
    /// The model's speaker key, matched against the last path segment of
    /// the acoustic config's `speakers` entries (`tiger_disco` matches
    /// `dsacoustic/tiger_disco` and `tiger_disco`).
    pub model_key: &'static str,
    /// An effect mode — growl, vocal fry, whisper: a colour for a line or
    /// a word, not a lead voice on a plain line (every one scored 1 in the
    /// 41-voice blind listen of 2026-10-13). Still selectable; the Speaker
    /// picker lists them after the other modes under "Effects", and no
    /// bank's default speaker is one.
    pub effect: bool,
}

/// How the shipped banks spell English phonemes when the bank is not
/// installed to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShippedSpelling {
    /// Bare ARPAbet (`ah`), no language input (TIGER, Lilia).
    Bare,
    /// `en/`-prefixed with a bare universal bucket (`AP SP hh cl`), and a
    /// per-token language id: English `en_id`, the bucket 0 (Meiji).
    Namespaced { en_id: i64 },
}

/// One row of the built-in table.
#[derive(Debug)]
pub struct BuiltinBank {
    pub id: &'static str,
    pub name: &'static str,
    /// Where the bank sits under a development models root
    /// (`RESONANCE_SVS_MODELS_DIR`, `resonance-svs/models/`); `None` for a
    /// bank that is only ever installed into the user voicebank folder.
    pub dev_dir: Option<&'static str>,
    /// The speakers to offer, in order; empty offers whatever the model
    /// declares.
    pub speakers: &'static [BuiltinSpeaker],
    /// The speaker a lane gets when it switches to this bank.
    pub default_speaker: fn(VoiceType) -> Option<&'static str>,
    /// Comfortable MIDI range `(low, high)` for a plain pop line: where
    /// the voice sits, not its extremes (no bank declares a range on disk;
    /// see the row comments for the sources).
    pub range: (u8, u8),
    /// Speakers whose comfortable range differs from `range`, by id.
    pub speaker_ranges: &'static [(&'static str, (u8, u8))],
    /// How far the timbre chip moves this bank's `gender` (formant) input:
    /// a factor on [`timbre_gender`](crate::compose::vocal_svs) values. TIGER
    /// loses its words at a |gender| of 0.15 (measured), so it gets a
    /// quarter of the swing.
    pub gender_scale: f32,
    /// The shipped English dictionary's `replacements:` (G2P symbol →
    /// bank symbol), for spelling phonemes while the bank is not installed;
    /// an installed bank reads its own.
    pub shipped_replacements: &'static [(&'static str, &'static str)],
    /// ARPAbet phones the bank lacks, with what to sing instead, most
    /// similar first. Tried before the generic voicing and articulatory
    /// tables.
    pub substitutions: &'static [(&'static str, &'static [&'static str])],
    pub credit: Option<&'static str>,
    pub terms_url: Option<&'static str>,
    /// `free_with_credit`, `paid` or `permission` (§5).
    pub commercial: Option<&'static str>,
    /// What the shipped config declares, for when the bank is missing.
    pub shipped_curves: CurveSupport,
    /// ARPAbet phones the shipped inventory lacks.
    pub shipped_missing: &'static [&'static str],
    pub shipped_spelling: ShippedSpelling,
}

/// TIGER's voice modes. Its acoustic model is shared with about 36 other
/// creators' speakers; only TIGER's own seven are offered (research note
/// `diffsinger_english_ecosystem.md` §1).
const TIGER_SPEAKERS: &[BuiltinSpeaker] = &[
    BuiltinSpeaker {
        id: "glam",
        name: "Glam",
        model_key: "tiger_glam",
        effect: false,
    },
    BuiltinSpeaker {
        id: "fresh",
        name: "Fresh",
        model_key: "tiger_fresh",
        effect: false,
    },
    BuiltinSpeaker {
        id: "disco",
        name: "Disco",
        model_key: "tiger_disco",
        effect: false,
    },
    BuiltinSpeaker {
        id: "royal",
        name: "Royal",
        model_key: "tiger_royal",
        effect: false,
    },
    BuiltinSpeaker {
        id: "electric",
        name: "Electric",
        model_key: "tiger_electric",
        effect: false,
    },
    BuiltinSpeaker {
        id: "mystic",
        name: "Mystic",
        model_key: "tiger_mystic",
        effect: false,
    },
    BuiltinSpeaker {
        id: "vinyl",
        name: "Vinyl",
        model_key: "tiger_vinyl",
        effect: false,
    },
];

/// Meiji's modes: character variants of one voice (neutral, strong,
/// mature, whispery). `phantom` ("whispery and husky", a whisper bank
/// turned into a mode) is not an effect mode: it sings a whole line
/// clearly (Whisper 100 %) and the blind listen scored it 3 as a lead,
/// above Meiji's own `standard`.
const MEIJI_SPEAKERS: &[BuiltinSpeaker] = &[
    BuiltinSpeaker {
        id: "standard",
        name: "Standard",
        model_key: "standard",
        effect: false,
    },
    BuiltinSpeaker {
        id: "hunter",
        name: "Hunter",
        model_key: "hunter",
        effect: false,
    },
    BuiltinSpeaker {
        id: "lilith",
        name: "Lilith",
        model_key: "lilith",
        effect: false,
    },
    BuiltinSpeaker {
        id: "phantom",
        name: "Phantom",
        model_key: "phantom",
        effect: false,
    },
];

/// Katyusha's modes, in her character file's order (LUNAI's site:
/// strong "powerful, intense", delicate "gentle, direct", lullaby "soft,
/// soothing").
const KATYUSHA_SPEAKERS: &[BuiltinSpeaker] = &[
    BuiltinSpeaker {
        id: "standard",
        name: "Standard",
        model_key: "standard",
        effect: false,
    },
    BuiltinSpeaker {
        id: "strong",
        name: "Strong",
        model_key: "strong",
        effect: false,
    },
    BuiltinSpeaker {
        id: "delicate",
        name: "Delicate",
        model_key: "delicate",
        effect: false,
    },
    BuiltinSpeaker {
        id: "lullaby",
        name: "Lullaby",
        model_key: "lullaby",
        effect: false,
    },
];

/// Keiro Revenant's modes, in his character file's order (`vf` is vocal
/// fry; native is his pre-2023 tone). Vocal fry, whisper and growl are
/// effect modes.
const KEIRO_SPEAKERS: &[BuiltinSpeaker] = &[
    BuiltinSpeaker {
        id: "standard",
        name: "Standard",
        model_key: "standard",
        effect: false,
    },
    BuiltinSpeaker {
        id: "native",
        name: "Native",
        model_key: "native",
        effect: false,
    },
    BuiltinSpeaker {
        id: "cold",
        name: "Cold",
        model_key: "cold",
        effect: false,
    },
    BuiltinSpeaker {
        id: "fragile",
        name: "Fragile",
        model_key: "fragile",
        effect: false,
    },
    BuiltinSpeaker {
        id: "mellow",
        name: "Mellow",
        model_key: "mellow",
        effect: false,
    },
    BuiltinSpeaker {
        id: "aggressive",
        name: "Aggressive",
        model_key: "aggressive",
        effect: false,
    },
    BuiltinSpeaker {
        id: "solid",
        name: "Solid",
        model_key: "solid",
        effect: false,
    },
    BuiltinSpeaker {
        id: "vf",
        name: "Vocal fry",
        model_key: "vf",
        effect: true,
    },
    BuiltinSpeaker {
        id: "whisper",
        name: "Whisper",
        model_key: "whisper",
        effect: true,
    },
    BuiltinSpeaker {
        id: "growl",
        name: "Growl",
        model_key: "growl",
        effect: true,
    },
];

/// Liam Thorne's modes, in his character file's order (`vf` is vocal fry).
/// Vocal fry and growl are effect modes.
const LIAM_SPEAKERS: &[BuiltinSpeaker] = &[
    BuiltinSpeaker {
        id: "standard",
        name: "Standard",
        model_key: "standard",
        effect: false,
    },
    BuiltinSpeaker {
        id: "gentle",
        name: "Gentle",
        model_key: "gentle",
        effect: false,
    },
    BuiltinSpeaker {
        id: "tired",
        name: "Tired",
        model_key: "tired",
        effect: false,
    },
    BuiltinSpeaker {
        id: "deep",
        name: "Deep",
        model_key: "deep",
        effect: false,
    },
    BuiltinSpeaker {
        id: "powerful",
        name: "Powerful",
        model_key: "powerful",
        effect: false,
    },
    BuiltinSpeaker {
        id: "aggressive",
        name: "Aggressive",
        model_key: "aggressive",
        effect: false,
    },
    BuiltinSpeaker {
        id: "rock",
        name: "Rock",
        model_key: "rock",
        effect: false,
    },
    BuiltinSpeaker {
        id: "crispy",
        name: "Crispy",
        model_key: "crispy",
        effect: false,
    },
    BuiltinSpeaker {
        id: "vf",
        name: "Vocal fry",
        model_key: "vf",
        effect: true,
    },
    BuiltinSpeaker {
        id: "growl",
        name: "Growl",
        model_key: "growl",
        effect: true,
    },
];

/// Allen Crow's modes, in his character file's order (LUNAI's site: open
/// and gentle are "intended for notes above C4"). Whisper is an effect
/// mode.
const ALLEN_SPEAKERS: &[BuiltinSpeaker] = &[
    BuiltinSpeaker {
        id: "standard",
        name: "Standard",
        model_key: "standard",
        effect: false,
    },
    BuiltinSpeaker {
        id: "heavy",
        name: "Heavy",
        model_key: "heavy",
        effect: false,
    },
    BuiltinSpeaker {
        id: "open",
        name: "Open",
        model_key: "open",
        effect: false,
    },
    BuiltinSpeaker {
        id: "gentle",
        name: "Gentle",
        model_key: "gentle",
        effect: false,
    },
    BuiltinSpeaker {
        id: "whisper",
        name: "Whisper",
        model_key: "whisper",
        effect: true,
    },
];

/// The TIGER mode each voice type historically sang with, so a lane
/// switched to TIGER keeps sounding the way it always did.
fn tiger_default_speaker(voice: VoiceType) -> Option<&'static str> {
    Some(match voice {
        VoiceType::Soprano => "glam",
        VoiceType::MezzoSoprano => "fresh",
        VoiceType::Alto => "disco",
        VoiceType::Tenor => "royal",
        VoiceType::Baritone => "electric",
        VoiceType::Bass => "mystic",
    })
}

/// Meiji's and the other LUNAI singers' default: their `standard` mode.
fn standard_speaker(_: VoiceType) -> Option<&'static str> {
    Some("standard")
}

fn no_default_speaker(_: VoiceType) -> Option<&'static str> {
    None
}

/// LIEE Lilia MM 2.8's English respellings (its `dsdict-en.yaml`).
const LILIA_REPLACEMENTS: &[(&str, &str)] = &[
    ("t", "t0"),
    ("d", "d0"),
    ("r", "r0"),
    ("l", "ll"),
    ("v", "w"),
    ("w", "L"),
    ("y", "j"),
];

/// LUNAI Project's terms of use (every LUNAI bank ships a copy, `terms of
/// use (English, Русский, 日本語).txt`, whose §6.1 names this as current).
const LUNAI_TERMS: &str = "https://lunaiproject.github.io/termsofuse/";

const TENSION_ONLY: CurveSupport = CurveSupport {
    energy: false,
    tension: true,
    breathiness: false,
};

/// The built-in table. Facts checked 2026-10-07 against the banks under
/// `resonance-svs/models/` and the research notes'
/// licensing section.
pub static BUILTIN_BANKS: &[BuiltinBank] = &[
    BuiltinBank {
        id: "tiger",
        name: "TIGER",
        dev_dir: Some("singer/extracted"),
        speakers: TIGER_SPEAKERS,
        default_speaker: tiger_default_speaker,
        // A male voice: A2..C#5 at the extremes (DiffSinger wiki, research
        // note `diffsinger_english_ecosystem.md` §1); a pop line sits
        // C3..A4. Every mode is the same singer. Whisper heard the audition
        // phrase equally well (90-95 %) centred anywhere from D3 to F4.
        range: (48, 69),
        speaker_ranges: &[],
        gender_scale: 0.25,
        shipped_replacements: &[],
        substitutions: &[],
        credit: Some("TIGER by tigermeat"),
        terms_url: Some("https://github.com/spicytigermeat/tiger_diffsinger"),
        commercial: Some("paid"),
        shipped_curves: CurveSupport {
            energy: false,
            tension: false,
            breathiness: false,
        },
        shipped_missing: &[],
        shipped_spelling: ShippedSpelling::Bare,
    },
    BuiltinBank {
        id: "lilia",
        name: "LIEE Lilia",
        dev_dir: Some("voicebanks/lilia"),
        speakers: &[],
        default_speaker: no_default_speaker,
        // Alto, optimal C3..G5 (UTAU wiki, research note
        // `diffsinger_english_ecosystem.md` §1). Her words blur as the
        // line climbs: Whisper heard the audition phrase 54 / 67 / 88 / 100 %
        // centred on A4 / E4 / B3 / F#3, so a line sits D3..A#4.
        range: (50, 70),
        speaker_ranges: &[],
        gender_scale: 1.0,
        // MM 2.8's multi-language inventory spells English with its own
        // symbols (`dsdur/dsdict-en.yaml`); English `v` is its `w`.
        shipped_replacements: LILIA_REPLACEMENTS,
        substitutions: &[],
        credit: Some("LIEE"),
        terms_url: Some(
            "https://docs.google.com/document/d/1t13WKc7VA2Q3G_MppznbAwhl3vLIL2MNPyjVN3kWDDg/edit",
        ),
        commercial: Some("free_with_credit"),
        shipped_curves: TENSION_ONLY,
        shipped_missing: &[],
        shipped_spelling: ShippedSpelling::Bare,
    },
    BuiltinBank {
        id: "meiji",
        name: "Gahata Meiji",
        dev_dir: Some("voicebanks/meiji"),
        speakers: MEIJI_SPEAKERS,
        default_speaker: standard_speaker,
        // A2..D5 at the extremes (research notes, which call the voice
        // male; LUNAI's own site describes Meiji's as a girl's voice); a
        // pop line sits B2..F#4. 2026-10-08: Whisper heard the audition
        // phrase 88 % centred on E4 or B3, 98 % on F#3. 2026-10-13, with the
        // bank's own models: both phrases 95-98 % centred anywhere from
        // C#3 to G4 (85 % on A#4), pitch within 50 cents on 88-97 % of the
        // notes from C#3 up (57-73 % on G2). Kept where the 12-voice
        // listen approved it.
        range: (47, 66),
        speaker_ranges: &[],
        gender_scale: 1.0,
        shipped_replacements: &[],
        substitutions: &[],
        credit: Some("Gahata Meiji from LUNAI Project"),
        terms_url: Some(LUNAI_TERMS),
        commercial: Some("permission"),
        shipped_curves: TENSION_ONLY,
        shipped_missing: &[],
        shipped_spelling: ShippedSpelling::Namespaced { en_id: 3 },
    },
    BuiltinBank {
        id: "katyusha",
        name: "Katyusha",
        dev_dir: None,
        speakers: KATYUSHA_SPEAKERS,
        default_speaker: standard_speaker,
        // Soprano / alto (research notes: the sjespersen/singer choir app's
        // pick for those parts). Transposition sweep 2026-10-13, every mode,
        // both phrases: Whisper 96-100 % centred from G2 to C#5, 86 % on
        // E5; pitch within 50 cents on 82-100 % of the notes centred from
        // E3 up, 36-60 % on G2 (her low notes miss). A line sits G3..D5.
        range: (55, 74),
        speaker_ranges: &[],
        gender_scale: 1.0,
        // Same `dsdict-en.yaml` replacements as Meiji: `en/`-prefixing
        // only (and `q` → `en/cl`), which the namespaced spelling does.
        shipped_replacements: &[],
        substitutions: &[],
        credit: Some("Katyusha from LUNAI Project"),
        terms_url: Some(LUNAI_TERMS),
        commercial: Some("permission"),
        shipped_curves: TENSION_ONLY,
        shipped_missing: &[],
        shipped_spelling: ShippedSpelling::Namespaced { en_id: 3 },
    },
    BuiltinBank {
        id: "keiro_revenant",
        name: "Keiro Revenant",
        dev_dir: None,
        speakers: KEIRO_SPEAKERS,
        default_speaker: standard_speaker,
        // Tenor (research notes). Sweep 2026-10-13, every mode, both
        // phrases: Whisper 94-100 % centred from G2 to A#4, 91 % on C#5,
        // 77 % on E5; pitch within 50 cents on 79-100 % of the notes from
        // E3 to A#4 (growl 27-57 % below E3; vocal fry 18-70 % anywhere —
        // fry is loose by nature). A line sits C3..A4.
        range: (48, 69),
        speaker_ranges: &[],
        gender_scale: 1.0,
        // Same `dsdict-en.yaml` replacements as Meiji: `en/`-prefixing
        // only (and `q` → `en/cl`), which the namespaced spelling does.
        shipped_replacements: &[],
        substitutions: &[],
        credit: Some("Keiro Revenant from LUNAI Project"),
        terms_url: Some(LUNAI_TERMS),
        commercial: Some("permission"),
        shipped_curves: TENSION_ONLY,
        shipped_missing: &[],
        shipped_spelling: ShippedSpelling::Namespaced { en_id: 3 },
    },
    BuiltinBank {
        id: "liam_thorne",
        name: "Liam Thorne",
        dev_dir: None,
        speakers: LIAM_SPEAKERS,
        default_speaker: standard_speaker,
        // Baritone / bass (research notes). Sweep 2026-10-13, every mode,
        // both phrases: Whisper 97-100 % centred from G2 to A#4, 89 % on
        // C#5, 79 % on E5; pitch within 50 cents on 82-100 % of the notes
        // from C#3 up (growl 39-67 % lower; vocal fry drops above C#4, to
        // 30-36 % on A#4). A line sits A2..F4.
        range: (45, 65),
        speaker_ranges: &[],
        gender_scale: 1.0,
        // Same `dsdict-en.yaml` replacements as Meiji: `en/`-prefixing
        // only (and `q` → `en/cl`), which the namespaced spelling does.
        shipped_replacements: &[],
        substitutions: &[],
        credit: Some("Liam Thorne from LUNAI Project"),
        terms_url: Some(LUNAI_TERMS),
        commercial: Some("permission"),
        shipped_curves: TENSION_ONLY,
        shipped_missing: &[],
        shipped_spelling: ShippedSpelling::Namespaced { en_id: 3 },
    },
    BuiltinBank {
        id: "allen_crow",
        name: "Allen Crow",
        dev_dir: None,
        speakers: ALLEN_SPEAKERS,
        default_speaker: standard_speaker,
        // A male voice (voice provider LOP-P, "the mirror reflection of
        // Llane Crow"); no range is published. LUNAI's site says open and
        // gentle are "intended for notes above C4". Sweep 2026-10-13, both
        // phrases: Whisper 94-99 % centred from G2 to A#4, 91 % on C#5,
        // 79 % on E5; pitch within 50 cents on 91-100 % of the notes from
        // E3 to C#5 for standard and heavy, while gentle (60 % on G2) and open
        // (76-88 % below E3) lose their low notes. A line sits C3..A4;
        // open and gentle a fifth higher, G3..D5 (Whisper 92-100 %, pitch
        // 88-100 % centred from A#3 to C#5).
        range: (48, 69),
        speaker_ranges: &[("open", (55, 74)), ("gentle", (55, 74))],
        gender_scale: 1.0,
        // Same `dsdict-en.yaml` replacements as Meiji: `en/`-prefixing
        // only (and `q` → `en/cl`), which the namespaced spelling does.
        shipped_replacements: &[],
        substitutions: &[],
        credit: Some("Allen Crow from LUNAI Project"),
        terms_url: Some(LUNAI_TERMS),
        commercial: Some("permission"),
        shipped_curves: TENSION_ONLY,
        shipped_missing: &[],
        shipped_spelling: ShippedSpelling::Namespaced { en_id: 3 },
    },
];

/// The built-in row for `id`, if Resonance knows the bank.
pub fn builtin(id: &str) -> Option<&'static BuiltinBank> {
    BUILTIN_BANKS.iter().find(|b| b.id == id)
}

/// Comfortable range for a bank nothing is known about: C3..E5, wide on
/// purpose (it catches a melody written an octave off, not taste).
pub const DEFAULT_RANGE: (u8, u8) = (48, 76);
