//! The built-in voice presets: one per voice Jorrit scored 3 or more (of
//! 5) in the 41-voice blind listen of 2026-10-13
//! (`docs/intents/compose/vocal-followups.md` item 1), best first.
//!
//! [`AUDITION_SCORES`] is that listen, every voice of it; [`ROWS`] names
//! the voices that became presets and gives each a name, a note and a
//! [`Character`]. The rest is derived, so a preset cannot drift from the
//! bank table it sings on:
//!
//! - **range**: the speaker's comfortable range from the built-in bank
//!   table (`registry::builtin`), exactly;
//! - **voice type**: the [`VoiceType`] whose default range is centred
//!   nearest that range's centre;
//! - **register tag**: `low` / `mid` / `high` from the same centre;
//! - **delivery**: a new lane's defaults — what every audition was
//!   rendered with — nudged by the mode's [`Character`] where the mode's
//!   name says something obvious (gentle sings softer, rock harder).
//!   Timbre stays `Warm` (it moves the model's formant input, and the
//!   listen judged the voices without it), vibrato rate and portamento
//!   stay at their defaults (the bank's own pitch model sings its own),
//!   and `bank_timing` / `bank_pitch` stay on.
//!
//! Order: score, then the bank's mean score over the whole listen, then
//! name.

use std::sync::OnceLock;

use resonance_music_theory::{VocalParams, VoiceType, VoicebankId};

use super::{VoicePreset, VoiceSettings};
use crate::compose::vocal_svs::registry::builtin::{builtin, DEFAULT_RANGE};

/// One voice of the blind listen: bank id, speaker id (`None` for a
/// single-voice bank) and its 1–5 score.
pub type AuditionScore = (&'static str, Option<&'static str>, u8);

/// The 41-voice blind listen of 2026-10-13 (Jorrit, 1–5, one phrase for
/// every voice, `scripts/vocal-audition-score.py`).
pub const AUDITION_SCORES: &[AuditionScore] = &[
    ("tiger", Some("mystic"), 5),
    ("tiger", Some("disco"), 4),
    ("tiger", Some("electric"), 4),
    ("tiger", Some("fresh"), 4),
    ("tiger", Some("royal"), 4),
    ("tiger", Some("vinyl"), 4),
    ("tiger", Some("glam"), 3),
    ("allen_crow", Some("heavy"), 4),
    ("allen_crow", Some("open"), 3),
    ("allen_crow", Some("standard"), 3),
    ("allen_crow", Some("gentle"), 2),
    ("allen_crow", Some("whisper"), 1),
    ("katyusha", Some("standard"), 4),
    ("katyusha", Some("delicate"), 3),
    ("katyusha", Some("strong"), 3),
    ("katyusha", Some("lullaby"), 2),
    ("keiro_revenant", Some("cold"), 4),
    ("keiro_revenant", Some("standard"), 4),
    ("keiro_revenant", Some("aggressive"), 3),
    ("keiro_revenant", Some("mellow"), 3),
    ("keiro_revenant", Some("native"), 3),
    ("keiro_revenant", Some("solid"), 3),
    ("keiro_revenant", Some("fragile"), 2),
    ("keiro_revenant", Some("growl"), 1),
    ("keiro_revenant", Some("vf"), 1),
    ("keiro_revenant", Some("whisper"), 1),
    ("liam_thorne", Some("deep"), 4),
    ("liam_thorne", Some("rock"), 4),
    ("liam_thorne", Some("standard"), 4),
    ("liam_thorne", Some("tired"), 4),
    ("liam_thorne", Some("aggressive"), 3),
    ("liam_thorne", Some("crispy"), 3),
    ("liam_thorne", Some("gentle"), 3),
    ("liam_thorne", Some("powerful"), 3),
    ("liam_thorne", Some("growl"), 1),
    ("liam_thorne", Some("vf"), 1),
    ("lilia", None, 3),
    ("meiji", Some("hunter"), 3),
    ("meiji", Some("lilith"), 3),
    ("meiji", Some("phantom"), 3),
    ("meiji", Some("standard"), 2),
];

/// The lowest score that makes a voice a built-in preset.
pub const MIN_PRESET_SCORE: u8 = 3;

/// The delivery a mode's name suggests. Modest on purpose: a nudge of a
/// few tenths on the sliders the name speaks to, never a new sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Character {
    /// A new lane's delivery, as auditioned.
    Neutral,
    /// gentle, delicate, mellow: less tension, softer dynamics, more legato.
    Gentle,
    /// powerful, strong, heavy, solid, hunter: more tension and dynamics.
    Powerful,
    /// aggressive, rock: the most tension and dynamics, shorter notes.
    Aggressive,
    /// tired: relaxed and still, little vibrato.
    Weary,
    /// cold: steady, little vibrato.
    Cool,
    /// deep: a little relaxed, a little more legato.
    Dark,
    /// Meiji's phantom ("whispery and husky"): breathy, almost no vibrato.
    Whispery,
    /// crispy: crisper consonants (inside the intelligible band).
    Crisp,
}

impl Character {
    /// The tag a preset of this character carries (none for neutral).
    pub fn tag(self) -> Option<&'static str> {
        match self {
            Self::Neutral => None,
            Self::Gentle => Some("gentle"),
            Self::Powerful => Some("powerful"),
            Self::Aggressive => Some("aggressive"),
            Self::Weary => Some("weary"),
            Self::Cool => Some("cool"),
            Self::Dark => Some("dark"),
            Self::Whispery => Some("whispery"),
            Self::Crisp => Some("crisp"),
        }
    }

    /// Apply this character's nudges to a new lane's delivery. The
    /// defaults it starts from: tension 0.5, tension from velocity 0.5 and
    /// from contour 0.5, vibrato 0.30, articulation 0.65, consonant
    /// emphasis 0.50, velocity → loudness 0.5.
    pub fn shape(self, s: &mut VoiceSettings) {
        match self {
            Self::Neutral => {}
            Self::Gentle => {
                s.tension = 0.2;
                s.tension_velocity_amount = 0.4;
                s.tension_contour_amount = 0.4;
                s.articulation = 0.55;
                s.velocity_dynamics = 0.4;
            }
            Self::Powerful => {
                s.tension = 0.7;
                s.tension_velocity_amount = 0.6;
                s.tension_contour_amount = 0.6;
                s.articulation = 0.7;
                s.velocity_dynamics = 0.6;
            }
            Self::Aggressive => {
                s.tension = 0.8;
                s.tension_velocity_amount = 0.7;
                s.tension_contour_amount = 0.6;
                s.vibrato = 0.25;
                s.articulation = 0.75;
                s.velocity_dynamics = 0.65;
            }
            Self::Weary => {
                s.tension = 0.3;
                s.tension_velocity_amount = 0.4;
                s.tension_contour_amount = 0.4;
                s.vibrato = 0.2;
                s.articulation = 0.6;
                s.velocity_dynamics = 0.4;
            }
            Self::Cool => {
                s.tension = 0.45;
                s.vibrato = 0.15;
            }
            Self::Dark => {
                s.tension = 0.4;
                s.articulation = 0.6;
            }
            Self::Whispery => {
                s.tension = 0.1;
                s.vibrato = 0.1;
                s.articulation = 0.6;
                s.velocity_dynamics = 0.4;
            }
            Self::Crisp => {
                s.articulation = 0.7;
                s.consonant_emphasis = 0.58;
            }
        }
    }
}

/// One built-in preset's hand-written part.
pub struct Row {
    pub bank: &'static str,
    pub speaker: Option<&'static str>,
    pub name: &'static str,
    /// A note on the mode (the bank creator's own words where there are
    /// some); the description adds the score and range.
    pub note: &'static str,
    pub character: Character,
    /// Tags beyond the register and the character's own.
    pub tags: &'static [&'static str],
}

const fn row(
    bank: &'static str,
    speaker: Option<&'static str>,
    name: &'static str,
    note: &'static str,
    character: Character,
    tags: &'static [&'static str],
) -> Row {
    Row {
        bank,
        speaker,
        name,
        note,
        character,
        tags,
    }
}

use Character::*;

/// Every voice scored [`MIN_PRESET_SCORE`] or more, in the listen's order
/// (the presets are sorted afterwards).
pub const ROWS: &[Row] = &[
    row("tiger", Some("mystic"), "TIGER Mystic", "TIGER's mystic mode, the voice a new lane sings", Neutral, &["mystic", "male"]),
    row("tiger", Some("disco"), "TIGER Disco", "TIGER's disco mode", Neutral, &["disco", "male"]),
    row("tiger", Some("electric"), "TIGER Electric", "TIGER's electric mode", Neutral, &["electric", "male"]),
    row("tiger", Some("fresh"), "TIGER Fresh", "TIGER's fresh mode", Neutral, &["fresh", "male"]),
    row("tiger", Some("royal"), "TIGER Royal", "TIGER's royal mode", Neutral, &["royal", "male"]),
    row("tiger", Some("vinyl"), "TIGER Vinyl", "TIGER's vinyl mode", Neutral, &["vinyl", "male"]),
    row("tiger", Some("glam"), "TIGER Glam", "TIGER's glam mode", Neutral, &["glam", "male"]),
    row("allen_crow", Some("heavy"), "Allen Crow \u{2014} Heavy tenor", "Allen Crow's heavy mode", Powerful, &["heavy", "male"]),
    row("allen_crow", Some("open"), "Allen Crow \u{2014} Open high voice", "Allen Crow's open mode, \"intended for notes above C4\" (LUNAI)", Neutral, &["open", "male"]),
    row("allen_crow", Some("standard"), "Allen Crow \u{2014} Tenor lead", "Allen Crow's standard mode", Neutral, &["standard", "male"]),
    row("katyusha", Some("standard"), "Katyusha \u{2014} Alto lead", "Katyusha's standard mode", Neutral, &["standard", "female"]),
    row("katyusha", Some("delicate"), "Katyusha \u{2014} Delicate alto", "Katyusha's delicate mode, \"gentle, direct\" (LUNAI)", Gentle, &["delicate", "female"]),
    row("katyusha", Some("strong"), "Katyusha \u{2014} Strong alto", "Katyusha's strong mode, \"powerful, intense\" (LUNAI)", Powerful, &["strong", "female"]),
    row("keiro_revenant", Some("cold"), "Keiro Revenant \u{2014} Cold tenor", "Keiro Revenant's cold mode", Cool, &["cold", "male"]),
    row("keiro_revenant", Some("standard"), "Keiro Revenant \u{2014} Tenor lead", "Keiro Revenant's standard mode", Neutral, &["standard", "male"]),
    row("keiro_revenant", Some("aggressive"), "Keiro Revenant \u{2014} Aggressive tenor", "Keiro Revenant's aggressive mode", Aggressive, &["male"]),
    row("keiro_revenant", Some("mellow"), "Keiro Revenant \u{2014} Mellow tenor", "Keiro Revenant's mellow mode", Gentle, &["mellow", "male"]),
    row("keiro_revenant", Some("native"), "Keiro Revenant \u{2014} Native tenor", "Keiro Revenant's native mode, his pre-2023 tone", Neutral, &["native", "male"]),
    row("keiro_revenant", Some("solid"), "Keiro Revenant \u{2014} Solid tenor", "Keiro Revenant's solid mode", Powerful, &["solid", "male"]),
    row("liam_thorne", Some("deep"), "Liam Thorne \u{2014} Deep baritone", "Liam Thorne's deep mode", Dark, &["deep", "male"]),
    row("liam_thorne", Some("rock"), "Liam Thorne \u{2014} Rock baritone", "Liam Thorne's rock mode", Aggressive, &["rock", "male"]),
    row("liam_thorne", Some("standard"), "Liam Thorne \u{2014} Baritone lead", "Liam Thorne's standard mode", Neutral, &["standard", "male"]),
    row("liam_thorne", Some("tired"), "Liam Thorne \u{2014} Tired baritone", "Liam Thorne's tired mode", Weary, &["tired", "male"]),
    row("liam_thorne", Some("aggressive"), "Liam Thorne \u{2014} Aggressive baritone", "Liam Thorne's aggressive mode", Aggressive, &["male"]),
    row("liam_thorne", Some("crispy"), "Liam Thorne \u{2014} Crisp baritone", "Liam Thorne's crispy mode", Crisp, &["crispy", "male"]),
    row("liam_thorne", Some("gentle"), "Liam Thorne \u{2014} Gentle baritone", "Liam Thorne's gentle mode", Gentle, &["male"]),
    row("liam_thorne", Some("powerful"), "Liam Thorne \u{2014} Powerful baritone", "Liam Thorne's powerful mode", Powerful, &["male"]),
    row("lilia", None, "LIEE Lilia", "LIEE Lilia, a single-voice alto bank", Neutral, &["female"]),
    row("meiji", Some("hunter"), "Gahata Meiji \u{2014} Hunter", "Meiji's hunter mode, the strong variant", Powerful, &["hunter"]),
    row("meiji", Some("lilith"), "Gahata Meiji \u{2014} Lilith", "Meiji's lilith mode, the mature variant", Neutral, &["lilith", "mature"]),
    row("meiji", Some("phantom"), "Gahata Meiji \u{2014} Phantom", "Meiji's phantom mode, \"whispery and husky\"", Whispery, &["phantom", "husky"]),
];

/// A built-in preset's id: `<bank>-<speaker>`, or the bank id alone for
/// a single-voice bank.
pub fn builtin_id(bank: &str, speaker: Option<&str>) -> String {
    match speaker {
        Some(s) => format!("{bank}-{s}"),
        None => bank.to_owned(),
    }
}

/// The audition score of `bank` / `speaker`, if the listen covered it.
pub fn audition_score(bank: &str, speaker: Option<&str>) -> Option<u8> {
    AUDITION_SCORES
        .iter()
        .find(|(b, s, _)| *b == bank && *s == speaker)
        .map(|(_, _, score)| *score)
}

/// A bank's mean score over every voice of it in the listen.
pub fn bank_mean_score(bank: &str) -> f32 {
    let scores: Vec<f32> = AUDITION_SCORES
        .iter()
        .filter(|(b, _, _)| *b == bank)
        .map(|(_, _, s)| f32::from(*s))
        .collect();
    if scores.is_empty() {
        return 0.0;
    }
    scores.iter().sum::<f32>() / scores.len() as f32
}

/// The voice type whose default range is centred nearest `range`'s centre.
pub fn nearest_voice_type(range: (u8, u8)) -> VoiceType {
    let centre = |(lo, hi): (u8, u8)| (f32::from(lo) + f32::from(hi)) / 2.0;
    let target = centre(range);
    VoiceType::ALL
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = (centre(a.default_range()) - target).abs();
            let db = (centre(b.default_range()) - target).abs();
            da.total_cmp(&db)
        })
        .unwrap_or(VoiceType::Tenor)
}

/// `low`, `mid` or `high`: where `range` sits (centre below G3 + ½, up to
/// C#4, above).
pub fn register_tag(range: (u8, u8)) -> &'static str {
    let centre = (f32::from(range.0) + f32::from(range.1)) / 2.0;
    if centre < 57.0 {
        "low"
    } else if centre <= 61.0 {
        "mid"
    } else {
        "high"
    }
}

/// The comfortable range of `bank`'s `speaker` from the built-in table.
fn comfortable_range(bank: &str, speaker: Option<&str>) -> (u8, u8) {
    let Some(b) = builtin(bank) else {
        return DEFAULT_RANGE;
    };
    speaker
        .and_then(|id| b.speaker_ranges.iter().find(|(s, _)| *s == id))
        .map(|(_, r)| *r)
        .unwrap_or(b.range)
}

fn note_name(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C\u{266F}", "D", "D\u{266F}", "E", "F", "F\u{266F}", "G", "G\u{266F}", "A",
        "A\u{266F}", "B",
    ];
    format!("{}{}", NAMES[usize::from(midi % 12)], i32::from(midi / 12) - 1)
}

fn build(row: &Row) -> VoicePreset {
    let score = audition_score(row.bank, row.speaker).unwrap_or(0);
    let range = comfortable_range(row.bank, row.speaker);
    let mut params = VocalParams::default();
    params.voicebank = VoicebankId::new(row.bank);
    params.speaker = row.speaker.map(str::to_owned);
    params.voice = nearest_voice_type(range);
    params.range = range;
    let mut settings = VoiceSettings::from_params(&params);
    row.character.shape(&mut settings);

    let mut tags = vec![register_tag(range).to_owned()];
    tags.extend(row.character.tag().map(str::to_owned));
    for tag in row.tags {
        if !tags.iter().any(|t| t == tag) {
            tags.push((*tag).to_owned());
        }
    }
    VoicePreset {
        id: builtin_id(row.bank, row.speaker),
        name: row.name.to_owned(),
        description: format!(
            "{}. Scored {score}/5 in the 41-voice blind listen; sings {}\u{2013}{}.",
            row.note,
            note_name(range.0),
            note_name(range.1)
        ),
        tags,
        score: Some(score),
        builtin: true,
        settings,
    }
}

/// The built-in presets, best first (built once).
pub fn builtin_presets() -> &'static [VoicePreset] {
    static PRESETS: OnceLock<Vec<VoicePreset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        let mut presets: Vec<(f32, VoicePreset)> = ROWS
            .iter()
            .map(|r| (bank_mean_score(r.bank), build(r)))
            .collect();
        presets.sort_by(|(mean_a, a), (mean_b, b)| {
            b.score
                .cmp(&a.score)
                .then(mean_b.total_cmp(mean_a))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        presets.into_iter().map(|(_, p)| p).collect()
    })
}
