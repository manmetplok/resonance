//! What Resonance knows about specific banks that their files do not say
//! (vocal-mcp-control-and-quality.md §4.3, D10): display names, which of
//! a bank's speakers to offer and what to call them, comfortable ranges,
//! phoneme substitution hints, terms of use — and, for the three banks
//! Resonance has shipped, the facts their configs declare, so a lane on a
//! bank that is not installed still validates and reports the way it will
//! once the bank is there.
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
    /// (`RESONANCE_SVS_MODELS_DIR`, `resonance-svs/models/`).
    pub dev_dir: &'static str,
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
    },
    BuiltinSpeaker {
        id: "fresh",
        name: "Fresh",
        model_key: "tiger_fresh",
    },
    BuiltinSpeaker {
        id: "disco",
        name: "Disco",
        model_key: "tiger_disco",
    },
    BuiltinSpeaker {
        id: "royal",
        name: "Royal",
        model_key: "tiger_royal",
    },
    BuiltinSpeaker {
        id: "electric",
        name: "Electric",
        model_key: "tiger_electric",
    },
    BuiltinSpeaker {
        id: "mystic",
        name: "Mystic",
        model_key: "tiger_mystic",
    },
    BuiltinSpeaker {
        id: "vinyl",
        name: "Vinyl",
        model_key: "tiger_vinyl",
    },
];

/// Meiji's modes: character variants of one voice (neutral, strong,
/// mature, whisper).
const MEIJI_SPEAKERS: &[BuiltinSpeaker] = &[
    BuiltinSpeaker {
        id: "standard",
        name: "Standard",
        model_key: "standard",
    },
    BuiltinSpeaker {
        id: "hunter",
        name: "Hunter",
        model_key: "hunter",
    },
    BuiltinSpeaker {
        id: "lilith",
        name: "Lilith",
        model_key: "lilith",
    },
    BuiltinSpeaker {
        id: "phantom",
        name: "Phantom",
        model_key: "phantom",
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

fn meiji_default_speaker(_: VoiceType) -> Option<&'static str> {
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
        dev_dir: "singer/extracted",
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
        dev_dir: "voicebanks/lilia",
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
        dev_dir: "voicebanks/meiji",
        speakers: MEIJI_SPEAKERS,
        default_speaker: meiji_default_speaker,
        // A male voice, A2..D5 at the extremes; a pop line sits B2..F#4
        // (Whisper heard the audition phrase 88 % centred on E4 or B3, 98 %
        // on F#3).
        range: (47, 66),
        speaker_ranges: &[],
        gender_scale: 1.0,
        shipped_replacements: &[],
        substitutions: &[],
        credit: Some("Gahata Meiji from LUNAI Project"),
        terms_url: Some("https://lunaiproject.github.io/termsofuse/"),
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
