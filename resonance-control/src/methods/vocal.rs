//! `vocal.*` — lyrics, pronunciation, and SVS rendering.
//!
//! # One lane per (section, track)
//!
//! A vocal lane must first be installed with `section.set_lane_generator`
//! (`kind = vocal`); nothing else creates one. Lyrics and the SVS voice
//! live per **(section definition, track)** vocal lane. The lane-addressed
//! mutations here take an optional `section_id` to pick one; omitting it
//! resolves the track's **first** vocal lane in placement order.
//! `song.vocal` lists a track's lanes with their ids, so a client can tell
//! which lane a write will hit.

use crate::ids::{ClipId, SectionDefinitionId, TrackId};
use serde::{Deserialize, Serialize};

/// `vocal.set_lyrics` — replace a vocal track's full lyric text
/// ([`SetLyricsParams`] -> `MutationAck`).
pub const SET_LYRICS: &str = "vocal.set_lyrics";
/// `vocal.set_line` — replace one lyric line
/// ([`SetLineParams`] -> `MutationAck`).
pub const SET_LINE: &str = "vocal.set_line";
/// `vocal.set_pronunciation` — set a per-word pronunciation override
/// ([`SetPronunciationParams`] -> `MutationAck`).
pub const SET_PRONUNCIATION: &str = "vocal.set_pronunciation";
/// `vocal.clear_pronunciation` — remove a per-word override
/// ([`ClearPronunciationParams`] -> `MutationAck`).
pub const CLEAR_PRONUNCIATION: &str = "vocal.clear_pronunciation";
/// `vocal.render` — kick off an SVS render
/// ([`RenderParams`] -> [`crate::job::JobStarted`]).
pub const RENDER: &str = "vocal.render";
/// `vocal.generate` — generate a vocal lane's melody (and lyrics) into
/// its derived clip ([`GenerateParams`] -> [`GenerateResult`]).
pub const GENERATE: &str = "vocal.generate";
/// `vocal.get_params` — read a vocal lane's full settings
/// ([`GetParamsParams`] -> [`ParamsResult`]).
pub const GET_PARAMS: &str = "vocal.get_params";
/// `vocal.set_params` — merge a partial patch into a vocal lane's
/// settings ([`SetParamsParams`] -> [`ParamsResult`]).
pub const SET_PARAMS: &str = "vocal.set_params";

/// All `vocal.*` method names.
pub const METHODS: &[&str] = &[
    SET_LYRICS,
    SET_LINE,
    SET_PRONUNCIATION,
    CLEAR_PRONUNCIATION,
    RENDER,
    GENERATE,
    GET_PARAMS,
    SET_PARAMS,
];

/// Params for `vocal.set_lyrics`: bulk text, one line per lyric line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetLyricsParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to write (`definition_id` from
    /// `song.vocal`'s `lanes`). Omitted resolves the track's first
    /// vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    /// Lyric text, one line per lyric line (`\n`-separated).
    ///
    /// Syllable breaks may be written explicitly as `-` or `·`
    /// (`re-so-lu-tion`); anything without a break is split for you (see
    /// [`Self::syllabify`]). One **syllable** sings on one note, so the
    /// break count is what decides how many notes a word needs.
    pub text: String,
    /// Split multi-syllable words across notes (default `true`).
    ///
    /// SVS sings one syllable per note. Stored verbatim, `"resolution"`
    /// is one syllable, so all nine of its phonemes are crammed onto a
    /// single note and the word is unintelligible. With this on, the
    /// engine's G2P inserts the missing `·` breaks (`re·so·lu·tion`) so
    /// each syllable gets its own note. Words you broke yourself (`-` or
    /// `·`) and `[..]` phoneme blocks are never touched.
    ///
    /// Set `false` only to store the text exactly as written — expect to
    /// place every break yourself.
    #[serde(default = "default_syllabify")]
    pub syllabify: bool,
}

fn default_syllabify() -> bool {
    true
}

/// Params for `vocal.set_line`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetLineParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to write (`definition_id` from
    /// `song.vocal`'s `lanes`). Omitted resolves the track's first
    /// vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    /// 0-based line index within the resolved lane (see `song.vocal`).
    pub line_index: usize,
    /// One lyric line. Same syllable-break rules as
    /// [`SetLyricsParams::text`].
    pub text: String,
    /// Split multi-syllable words across notes (default `true`). See
    /// [`SetLyricsParams::syllabify`].
    #[serde(default = "default_syllabify")]
    pub syllabify: bool,
}

/// Params for `vocal.set_pronunciation`. Pronunciation overrides are
/// project-wide, not per lane, so this takes no `section_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetPronunciationParams {
    /// The word being overridden (case-insensitive).
    pub word: String,
    /// Lowercase phonemes, e.g. `["l", "ih", "l", "iy", "ah"]`.
    pub phonemes: Vec<String>,
}

/// Params for `vocal.clear_pronunciation`. Project-wide, like
/// [`SetPronunciationParams`] — no `section_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ClearPronunciationParams {
    pub word: String,
}

/// Params for `vocal.render`. A render covers **every lane it names**:
/// `section_id` picks exactly one, `track_id` alone covers all of that
/// track's vocal lanes, and omitting both covers every vocal lane in the
/// project. The job resolves only once all of them have rendered; its
/// payload is [`RenderJobResult`].
///
/// Render synthesises the notes already in the lane's clip; it never
/// generates or rewrites them (`vocal.generate` does that).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RenderParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    /// Which of the track's vocal lanes to render. Omitted renders
    /// **every** vocal lane on the track — unlike the rest of the
    /// namespace, where an omitted section means the track's first lane.
    /// Resolving a single lane here left a track that sings in several
    /// sections frozen on its previous audio everywhere but its first
    /// lane, while the call still reported success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    /// Voicebank id (`tiger`, `lilia` or `meiji`; matched
    /// case-insensitively), stored on every lane the render covers.
    /// Omitted renders each lane with the bank it already has — a plain
    /// re-render never changes it. Prefer `vocal.set_params` to choose a
    /// bank; this is a shortcut for "switch and render".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voicebank: Option<String>,
}

/// Params for `vocal.generate`.
///
/// `generate.part` refuses vocal tracks (drums have their own method,
/// vocals their own namespace), and vocal rendering reads its notes from
/// the lane's **derived clip** — so without this method a vocal lane had
/// no notes and `vocal.render` had nothing to sing (doc #269 FR-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GenerateParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to generate. Omitted resolves
    /// the track's first vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    /// Explicit RNG seed for a reproducible result; omitted advances the
    /// lane's seed, so repeated calls give different material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Also roll a fresh lyric draft from the lane's theme brief.
    /// Defaults to `true`, matching the GUI's "generate" button. Pass
    /// `false` to generate the **melody only** and leave lyrics you
    /// wrote with `vocal.set_lyrics` untouched — generation writes both
    /// by default, so doing it the other way round silently discards
    /// them.
    #[serde(default = "default_lyrics")]
    pub lyrics: bool,
}

fn default_lyrics() -> bool {
    true
}

/// Result of `vocal.generate`: the lane's derived MIDI clip, ready for
/// `song.notes` / `notes.*` edits and then `vocal.render`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GenerateResult {
    /// The derived clip at the lane's first placement. A lane derives
    /// one clip per placement of its section, all carrying the same
    /// material.
    pub clip_id: ClipId,
    pub revision: u64,
}

/// Job payload once a `vocal.render` job completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RenderJobResult {
    /// Tracks that were (re)rendered.
    pub track_ids: Vec<TrackId>,
    pub revision: u64,
}

// ---------------------------------------------------------------------------
// Lane params (`vocal.get_params` / `vocal.set_params`)
// ---------------------------------------------------------------------------

/// Params for `vocal.get_params`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct GetParamsParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to read (`definition_id` from
    /// `song.vocal`'s `lanes`). Omitted resolves the track's first
    /// vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
}

/// Params for `vocal.set_params`.
///
/// `params` is a **partial** patch: every field is optional, the fields
/// present merge into the lane's current settings and the rest are kept.
/// The merged result is validated before anything changes, so one bad
/// field rejects the whole call (`invalid_params` naming the field). One
/// call is one undo entry. The call never renders and never regenerates:
/// follow it with `vocal.generate` (melody/lyric generator fields) or
/// `vocal.render` (voice fields) to hear the change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetParamsParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to write (`definition_id` from
    /// `song.vocal`'s `lanes`). Omitted resolves the track's first
    /// vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    pub params: VocalParamsPatch,
}

/// Result of `vocal.get_params` and `vocal.set_params`: the lane's full
/// settings after the call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ParamsResult {
    pub track_id: TrackId,
    /// The lane's section definition (pass as `section_id` to address it).
    pub section_id: SectionDefinitionId,
    pub params: VocalParamsView,
    /// The lane's generator seed (`vocal.generate` advances it).
    pub seed: u64,
    pub revision: u64,
}

/// Vocal tessitura preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalVoice {
    Soprano,
    MezzoSoprano,
    Alto,
    Tenor,
    Baritone,
    Bass,
}

/// Melodic style of the melody generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalStyle {
    PopBallad,
    Conversational,
    Hymnal,
    Folk,
    Anthemic,
    Chant,
}

/// Phrase contour of the melody generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalContour {
    Arch,
    Rise,
    Fall,
    Wave,
    Flat,
}

/// Note-to-syllable mapping of the melody generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalSyllableMode {
    Syllabic,
    Mixed,
    Melismatic,
}

/// Mood of the lyric generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalMood {
    Yearning,
    Defiant,
    Hopeful,
    Reflective,
    Joyful,
    Melancholy,
}

/// Point of view of the lyric generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalPov {
    FirstSingular,
    FirstPlural,
    SecondPerson,
    ThirdPerson,
    Narrator,
}

/// End-rhyme scheme of the lyric generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalRhyme {
    Aabb,
    Abab,
    Abcb,
    Abba,
    Free,
}

/// Timbre preset of the voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalTimbre {
    Airy,
    Warm,
    Edged,
    Bright,
}

/// A melody range in MIDI notes (60 = C4), as sent in a patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VocalRangePatch {
    pub low: u8,
    pub high: u8,
}

/// A melody range in MIDI notes (60 = C4), as reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VocalRange {
    pub low: u8,
    pub high: u8,
}

/// A partial update of a vocal lane's settings. Every field is optional;
/// absent fields keep the lane's current value. Lyrics are not here (use
/// `vocal.set_lyrics` / `vocal.set_line`).
///
/// Ranges are checked on the merged result: the 0..1 fields
/// (`chord_tone_anchor`, `leap_range`, `breath`, `vibrato`,
/// `articulation`, `consonant_emphasis`, `tension_velocity_amount`,
/// `tension_contour_amount`), `tension` -1..1, `vibrato_rate` 1..20 Hz,
/// `portamento_ms` 0..500, `lines` >= 1, `phrase_length_bars` >= 1,
/// `1 <= syllables_min <= syllables_max`, `range.low < range.high <= 127`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VocalParamsPatch {
    // ---- Voice / render ----
    /// Voicebank id: `tiger`, `lilia` (the default) or `meiji`. Changing
    /// it resets `speaker` to that bank's default unless the same patch
    /// sets `speaker` too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voicebank: Option<String>,
    /// The bank's singer: on `tiger` one of `glam`, `fresh`, `disco`,
    /// `royal`, `electric`, `mystic`, `vinyl`; on `meiji` one of
    /// `standard`, `hunter`, `lilith`, `phantom`. `lilia` has a single
    /// voice and refuses a speaker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timbre: Option<VocalTimbre>,
    /// Vibrato depth, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<f32>,
    /// Vibrato rate in Hz, 1..20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibrato_rate: Option<f32>,
    /// Baseline tension, -1 (breathy) .. +1 (belted). Not heard on `tiger`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tension: Option<f32>,
    /// How much note velocity modulates tension, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tension_velocity_amount: Option<f32>,
    /// How much the pitch contour modulates tension, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tension_contour_amount: Option<f32>,
    /// Note-to-note glide in milliseconds, 0..500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portamento_ms: Option<f32>,
    /// 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub articulation: Option<f32>,
    /// Consonant length, 0..1; 0.4..0.6 is the intelligible band.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consonant_emphasis: Option<f32>,

    // ---- Melody generator ----
    /// Voice type. Changing it does not move `range`; set both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VocalVoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<VocalRangePatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<VocalStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contour: Option<VocalContour>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syllable_mode: Option<VocalSyllableMode>,
    /// Probability a strong-beat syllable lands on a chord tone, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chord_tone_anchor: Option<f32>,
    /// Leap-vs-step bias, 0 (always step) .. 1 (always leap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leap_range: Option<f32>,
    /// Bars per melodic phrase, >= 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase_length_bars: Option<u8>,
    /// Breath gap between phrases, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breath: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stay_in_scale: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoid_clashes: Option<bool>,
    /// Reuse the section's shared motif intervals for the melody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_section_motif: Option<bool>,

    // ---- Lyric generator ----
    /// Theme brief the lyric generator writes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mood: Option<VocalMood>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pov: Option<VocalPov>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rhyme: Option<VocalRhyme>,
    /// Number of lyric lines to generate, >= 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syllables_min: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syllables_max: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_syllables_to_melody: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoid_cliches: Option<bool>,
}

/// A vocal lane's full settings — the shape [`VocalParamsPatch`] edits,
/// with every field present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VocalParamsView {
    // ---- Voice / render ----
    /// Voicebank id: `tiger`, `lilia` or `meiji`.
    pub voicebank: String,
    /// The bank's singer; absent on a single-voice bank (`lilia`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub timbre: VocalTimbre,
    pub vibrato: f32,
    pub vibrato_rate: f32,
    pub tension: f32,
    pub tension_velocity_amount: f32,
    pub tension_contour_amount: f32,
    pub portamento_ms: f32,
    pub articulation: f32,
    pub consonant_emphasis: f32,

    // ---- Melody generator ----
    pub voice: VocalVoice,
    pub range: VocalRange,
    pub style: VocalStyle,
    pub contour: VocalContour,
    pub syllable_mode: VocalSyllableMode,
    pub chord_tone_anchor: f32,
    pub leap_range: f32,
    pub phrase_length_bars: u8,
    pub breath: f32,
    pub stay_in_scale: bool,
    pub avoid_clashes: bool,
    pub use_section_motif: bool,

    // ---- Lyric generator ----
    pub theme: String,
    pub mood: VocalMood,
    pub pov: VocalPov,
    pub rhyme: VocalRhyme,
    pub lines: u8,
    pub syllables_min: u8,
    pub syllables_max: u8,
    pub match_syllables_to_melody: bool,
    pub avoid_cliches: bool,
}
