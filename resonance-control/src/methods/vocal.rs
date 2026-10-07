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
/// `vocal.voicebanks` — the installed voicebanks and what each can do,
/// and the plok.org index's banks that are not installed (no params ->
/// [`VoicebankList`]). Read-only, needs no project.
pub const VOICEBANKS: &str = "vocal.voicebanks";
/// `vocal.voicebank_install` — download a voicebank from the plok.org
/// index (the creator's own URL) and its vocoder when it needs one, as a
/// job ([`VoicebankInstallParams`] -> [`crate::job::JobStarted`]; the
/// job's result is a [`VoicebankInstallResult`]). Needs no project; no
/// undo entry, no revision bump.
pub const VOICEBANK_INSTALL: &str = "vocal.voicebank_install";

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
    VOICEBANKS,
    VOICEBANK_INSTALL,
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
    /// [`Self::syllabify`]). Each **syllable** starts its own note, so the
    /// break count is what decides how many notes a word needs. Two
    /// standalone markers change that: `_` (alias `+`) holds the previous
    /// syllable onto one more note (a melisma — `love _ _` is one syllable
    /// over three notes), and an uppercase `AP` marks a breath before the
    /// next word and takes no note (not yet rendered as a breath sound).
    pub text: String,
    /// Split multi-syllable words across notes (default `true`).
    ///
    /// A syllable starts one note. Stored verbatim, `"resolution"`
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
    /// Voicebank id (an installed bank's id from `vocal.voicebanks`, e.g.
    /// `tiger`, `lilia`, `meiji`; matched case-insensitively), stored on
    /// every lane the render covers.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GenerateResult {
    /// The derived clip at the lane's first placement. A lane derives
    /// one clip per placement of its section, all carrying the same
    /// material.
    pub clip_id: ClipId,
    pub revision: u64,
    /// Set when the melody was generated but its render cannot run: the
    /// lane's voicebank (or its vocoder) is not installed
    /// (`kind: voicebank_missing`). The notes are there; install the bank
    /// or pick another with `vocal.set_params`, then `vocal.render`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_error: Option<RenderError>,
}

/// Why a render did not start: a typed kind plus the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RenderError {
    pub kind: crate::rpc::ErrorKind,
    pub message: String,
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
/// `tension_contour_amount`, `velocity_dynamics`), `tension` -1..1,
/// `vibrato_rate` 1..20 Hz, `portamento_ms` 0..500,
/// `rest_threshold_beats` 0.125..4, `lines` >= 1, `phrase_length_bars` >= 1,
/// `1 <= syllables_min <= syllables_max`, `range.low < range.high <= 127`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VocalParamsPatch {
    // ---- Voice / render ----
    /// Voicebank id: any installed bank's id (`vocal.voicebanks` lists
    /// them), or one of the known `tiger`, `lilia` (the default),
    /// `meiji`. A bank that cannot sing English is refused. Changing it
    /// resets `speaker` to that bank's default unless the same patch sets
    /// `speaker` too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voicebank: Option<String>,
    /// The bank's speaker (voice mode), one of its `speakers` ids from
    /// `vocal.voicebanks`: on `tiger` `glam`, `fresh`, `disco`, `royal`,
    /// `electric`, `mystic`, `vinyl`; on `meiji` `standard`, `hunter`,
    /// `lilith`, `phantom`. A single-voice bank (`lilia`) refuses one.
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
    /// Shortest gap between two notes, in beats at the lane's tempo, that
    /// is sung as a rest (silence); a shorter gap is sung legato.
    /// 0.125..4, default 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_threshold_beats: Option<f32>,
    /// How much note velocity drives loudness, 0..1 (default 0.5): the
    /// bank's energy curve where it has one, and a per-note gain of up to
    /// +/-6 dB after the vocoder on every bank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_dynamics: Option<f32>,

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
    /// Voicebank id (`vocal.voicebanks` lists the installed ones).
    pub voicebank: String,
    /// The bank's speaker; absent on a single-voice bank (`lilia`).
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
    /// Shortest gap, in beats, sung as a rest.
    pub rest_threshold_beats: f32,
    /// Velocity -> loudness amount, 0..1.
    pub velocity_dynamics: f32,

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

// ---------------------------------------------------------------------------
// Voicebanks (`vocal.voicebanks`)
// ---------------------------------------------------------------------------

/// Result of `vocal.voicebanks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VoicebankList {
    /// The banks installed on this machine, by id.
    pub installed: Vec<VoicebankEntry>,
    /// Banks the plok.org index offers that are not installed (install
    /// one with `vocal.voicebank_install`).
    #[serde(default)]
    pub available: Vec<AvailableVoicebank>,
    /// Where `available` comes from: `fresh` (fetched in the last 10
    /// minutes), `cached` (an older or on-disk copy; a refresh is under
    /// way), `fetching` (none yet; call again shortly) or `unavailable`
    /// (no index could be read; `index_error` says why).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_error: Option<String>,
}

/// One installed voicebank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VoicebankEntry {
    /// The id `vocal.set_params` takes as `voicebank`.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Can sing English (lyrics are English); `false` banks are refused
    /// by `vocal.set_params`.
    pub english: bool,
    /// The speakers (voice modes) `speaker` takes; empty for a
    /// single-voice bank.
    pub speakers: Vec<VoicebankSpeaker>,
    /// The speaker a lane gets on switching to this bank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_speaker: Option<String>,
    /// Where the bank sings clearly.
    pub comfortable_range: VoicebankRange,
    /// ARPAbet phonemes the bank lacks.
    pub missing_phonemes: Vec<String>,
    /// What each missing phoneme is sung as; a missing phoneme without
    /// an entry cannot be sung and blocks a render that needs it.
    pub substitutions: Vec<PhonemeSubstitution>,
    /// Which expression curves reach the model.
    pub curves: VoicebankCurves,
    /// The credit line the bank's terms ask for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
    /// `free_with_credit`, `paid` or `permission`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commercial: Option<String>,
    /// The vocoder the bank renders with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocoder: Option<String>,
    /// `false`: the vocoder is missing, and rendering this bank fails
    /// with `voicebank_missing`.
    pub vocoder_installed: bool,
}

/// One speaker of a voicebank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VoicebankSpeaker {
    pub id: String,
    pub name: String,
    pub comfortable_range: VoicebankRange,
}

/// A comfortable MIDI range (inclusive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VoicebankRange {
    pub low: u8,
    pub high: u8,
    /// `builtin` (Resonance's estimate for this bank) or `default`
    /// (nothing is known: a wide C3..E5).
    pub range_source: String,
}

/// A phoneme a bank lacks and what it is sung as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PhonemeSubstitution {
    pub phoneme: String,
    pub sung_as: String,
}

/// Which expression curves a bank's acoustic model takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VoicebankCurves {
    pub dynamics: bool,
    pub tension: bool,
    pub breathiness: bool,
    /// Always `true`: pitch bend is applied before synthesis.
    pub pitch_bend: bool,
}

/// A downloadable voicebank from the plok.org index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AvailableVoicebank {
    /// The id `vocal.voicebank_install` takes (and the bank's id once
    /// installed).
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commercial: Option<String>,
    /// Language codes the bank sings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Its voice modes, for display.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<String>,
    /// Download size ("609.3 MB").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// The shared vocoder it needs (fetched with it) when it ships none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocoder: Option<String>,
    /// The creator's download URL the install fetches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Params for `vocal.voicebank_install`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VoicebankInstallParams {
    /// The bank's id from `vocal.voicebanks`' `available` (or an installed
    /// bank's id, to update it to the index's version).
    pub id: String,
}

/// The result of a finished `vocal.voicebank_install` job. Running the
/// install accepts the bank's terms on the user's behalf: tell the user
/// the credit line, the terms URL and the commercial-use summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VoicebankInstallResult {
    /// The installed bank's id (`vocal.set_params` takes it as
    /// `voicebank`).
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    /// `free_with_credit`, `paid` or `permission`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commercial: Option<String>,
    /// The vocoder the bank renders with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocoder: Option<String>,
    /// `false`: the bank names a vocoder that is neither installed nor in
    /// the index, so rendering it fails with `voicebank_missing`.
    pub vocoder_installed: bool,
    /// This install downloaded the shared vocoder too.
    #[serde(default)]
    pub vocoder_fetched: bool,
    /// The bank was already installed as the index lists it; nothing was
    /// downloaded.
    #[serde(default)]
    pub already_installed: bool,
}
