//! How a [`VocalParams`] is read back from a project file or template:
//! the current shape plus the fields older files carried
//! (vocal-mcp-control-and-quality.md §4.1).
//!
//! Older files stored the bank as an enum name (`"Tiger"`, `"Lilia"`,
//! `"Meiji"`) and the speaker in two per-bank fields: `singer` (TIGER's
//! `Glam` … `Vinyl`) and `singer_meiji` (Meiji's `Standard` … `Phantom`),
//! both always present whatever the bank. [`VoicebankId`] lowercases the
//! enum names into the ids they became, and the speaker comes from the
//! field that belonged to the lane's bank, lowercased (`Disco` → `disco`).
//! A file that already has `speaker` keeps it.
//!
//! Every field's `#[serde(default …)]` lives here: `VocalParams` reads
//! through this type (`#[serde(from)]`), so its own attributes would only
//! shape writing.

use serde::Deserialize;

use super::super::lyrics::LyricLine;
use super::{
    default_portamento_ms, default_rest_threshold_beats, default_tension,
    default_tension_contour_amount, default_tension_velocity_amount, default_velocity_dynamics,
    default_vibrato_rate, default_vocal_style, SyllableMode,
    VocalContour, VocalMood, VocalParams, VocalPov, VocalRhymeScheme, VocalStyle, VocalTimbre,
    VoiceType, VoicebankId,
};

#[derive(Deserialize)]
pub(super) struct VocalParamsRepr {
    theme: String,
    mood: VocalMood,
    pov: VocalPov,
    rhyme: VocalRhymeScheme,
    lines: u8,
    syllables_min: u8,
    syllables_max: u8,
    match_syllables_to_melody: bool,
    avoid_cliches: bool,
    #[serde(default)]
    draft: Vec<LyricLine>,
    voice: VoiceType,
    range: (u8, u8),
    #[serde(default = "default_vocal_style")]
    style: VocalStyle,
    contour: VocalContour,
    syllable_mode: SyllableMode,
    chord_tone_anchor: f32,
    leap_range: f32,
    phrase_length_bars: u8,
    breath: f32,
    stay_in_scale: bool,
    avoid_clashes: bool,
    #[serde(default)]
    use_section_motif: bool,
    timbre: VocalTimbre,
    vibrato: f32,
    #[serde(default = "default_vibrato_rate")]
    vibrato_rate: f32,
    #[serde(default = "default_tension")]
    tension: f32,
    #[serde(default = "default_tension_velocity_amount")]
    tension_velocity_amount: f32,
    #[serde(default = "default_tension_contour_amount")]
    tension_contour_amount: f32,
    #[serde(default = "default_portamento_ms")]
    portamento_ms: f32,
    articulation: f32,
    consonant_emphasis: f32,
    #[serde(default = "default_rest_threshold_beats")]
    rest_threshold_beats: f32,
    #[serde(default = "default_velocity_dynamics")]
    velocity_dynamics: f32,
    #[serde(default)]
    speaker: Option<String>,
    #[serde(default)]
    voicebank: VoicebankId,
    /// Pre-registry TIGER speaker (`"Glam"` …). Read, never written.
    #[serde(default)]
    singer: Option<String>,
    /// Pre-registry Meiji mode (`"Standard"` …). Read, never written.
    #[serde(default)]
    singer_meiji: Option<String>,
}

/// TIGER's speaker when an old file had no `singer` either: the
/// `VoiceType::Alto` default the field used to fall back to.
const LEGACY_TIGER_DEFAULT: &str = "disco";
/// Meiji's speaker when an old file had no `singer_meiji`.
const LEGACY_MEIJI_DEFAULT: &str = "standard";

impl From<VocalParamsRepr> for VocalParams {
    fn from(r: VocalParamsRepr) -> Self {
        let lower = |s: String| s.trim().to_ascii_lowercase();
        let speaker = match r.speaker {
            Some(s) => Some(s),
            None => match r.voicebank.as_str() {
                "tiger" => Some(r.singer.map_or(LEGACY_TIGER_DEFAULT.to_owned(), lower)),
                "meiji" => Some(
                    r.singer_meiji
                        .map_or(LEGACY_MEIJI_DEFAULT.to_owned(), lower),
                ),
                _ => None,
            },
        };
        VocalParams {
            theme: r.theme,
            mood: r.mood,
            pov: r.pov,
            rhyme: r.rhyme,
            lines: r.lines,
            syllables_min: r.syllables_min,
            syllables_max: r.syllables_max,
            match_syllables_to_melody: r.match_syllables_to_melody,
            avoid_cliches: r.avoid_cliches,
            draft: r.draft,
            voice: r.voice,
            range: r.range,
            style: r.style,
            contour: r.contour,
            syllable_mode: r.syllable_mode,
            chord_tone_anchor: r.chord_tone_anchor,
            leap_range: r.leap_range,
            phrase_length_bars: r.phrase_length_bars,
            breath: r.breath,
            stay_in_scale: r.stay_in_scale,
            avoid_clashes: r.avoid_clashes,
            use_section_motif: r.use_section_motif,
            timbre: r.timbre,
            vibrato: r.vibrato,
            vibrato_rate: r.vibrato_rate,
            tension: r.tension,
            tension_velocity_amount: r.tension_velocity_amount,
            tension_contour_amount: r.tension_contour_amount,
            portamento_ms: r.portamento_ms,
            articulation: r.articulation,
            consonant_emphasis: r.consonant_emphasis,
            rest_threshold_beats: r.rest_threshold_beats,
            velocity_dynamics: r.velocity_dynamics,
            speaker,
            voicebank: r.voicebank,
        }
    }
}
