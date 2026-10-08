//! Vocal lane settings <-> the snake_case control wire
//! (vocal-mcp-control-and-quality.md §3.2).
//!
//! The wire has its own enums (`resonance_control::methods::vocal`) so
//! `VocalParams`' serde — the project format — never changes when the
//! wire does. This module owns both directions of the enum mapping and
//! the full [`vocal_params_view`]; the patch merge that uses them is
//! `update::control::vocal::apply_params_patch`. Voicebank and speaker
//! are string ids on both sides, checked against the voicebank registry.

use resonance_control::methods::vocal as wire;
use resonance_music_theory::{
    SyllableMode, VocalContour, VocalMood, VocalParams, VocalPov, VocalRhymeScheme, VocalStyle,
    VocalTimbre, VoiceType,
};

/// One wire enum <-> one app enum, variant for variant (the variant
/// names match on both sides; only the serde spelling differs).
macro_rules! enum_bridge {
    ($to_wire:ident, $from_wire:ident, $app:ident, $wire:ident, [$($v:ident),+ $(,)?]) => {
        pub(in crate::update::control) fn $to_wire(v: $app) -> wire::$wire {
            match v {
                $($app::$v => wire::$wire::$v,)+
            }
        }

        pub(in crate::update::control) fn $from_wire(v: wire::$wire) -> $app {
            match v {
                $(wire::$wire::$v => $app::$v,)+
            }
        }
    };
}

enum_bridge!(voice_to_wire, voice_from_wire, VoiceType, VocalVoice,
    [Soprano, MezzoSoprano, Alto, Tenor, Baritone, Bass]);
enum_bridge!(style_to_wire, style_from_wire, VocalStyle, VocalStyle,
    [PopBallad, Conversational, Hymnal, Folk, Anthemic, Chant]);
enum_bridge!(contour_to_wire, contour_from_wire, VocalContour, VocalContour,
    [Arch, Rise, Fall, Wave, Flat]);
enum_bridge!(syllable_mode_to_wire, syllable_mode_from_wire, SyllableMode, VocalSyllableMode,
    [Syllabic, Mixed, Melismatic]);
enum_bridge!(mood_to_wire, mood_from_wire, VocalMood, VocalMood,
    [Yearning, Defiant, Hopeful, Reflective, Joyful, Melancholy]);
enum_bridge!(pov_to_wire, pov_from_wire, VocalPov, VocalPov,
    [FirstSingular, FirstPlural, SecondPerson, ThirdPerson, Narrator]);
enum_bridge!(rhyme_to_wire, rhyme_from_wire, VocalRhymeScheme, VocalRhyme,
    [Aabb, Abab, Abcb, Abba, Free]);
enum_bridge!(timbre_to_wire, timbre_from_wire, VocalTimbre, VocalTimbre,
    [Airy, Warm, Edged, Bright]);

/// A lane's full settings in wire shape (`vocal.get_params`, and each
/// `song.vocal` lane's `params`).
pub(in crate::update::control) fn vocal_params_view(p: &VocalParams) -> wire::VocalParamsView {
    wire::VocalParamsView {
        voicebank: p.voicebank.to_string(),
        speaker: p.speaker.clone(),
        timbre: timbre_to_wire(p.timbre),
        vibrato: p.vibrato,
        vibrato_rate: p.vibrato_rate,
        tension: p.tension,
        tension_velocity_amount: p.tension_velocity_amount,
        tension_contour_amount: p.tension_contour_amount,
        portamento_ms: p.portamento_ms,
        articulation: p.articulation,
        consonant_emphasis: p.consonant_emphasis,
        rest_threshold_beats: p.rest_threshold_beats,
        velocity_dynamics: p.velocity_dynamics,
        bank_timing: p.bank_timing,
        bank_pitch: p.bank_pitch,
        voice: voice_to_wire(p.voice),
        range: wire::VocalRange {
            low: p.range.0,
            high: p.range.1,
        },
        style: style_to_wire(p.style),
        contour: contour_to_wire(p.contour),
        syllable_mode: syllable_mode_to_wire(p.syllable_mode),
        chord_tone_anchor: p.chord_tone_anchor,
        leap_range: p.leap_range,
        phrase_length_bars: p.phrase_length_bars,
        breath: p.breath,
        stay_in_scale: p.stay_in_scale,
        avoid_clashes: p.avoid_clashes,
        use_section_motif: p.use_section_motif,
        theme: p.theme.clone(),
        mood: mood_to_wire(p.mood),
        pov: pov_to_wire(p.pov),
        rhyme: rhyme_to_wire(p.rhyme),
        lines: p.lines,
        syllables_min: p.syllables_min,
        syllables_max: p.syllables_max,
        match_syllables_to_melody: p.match_syllables_to_melody,
        avoid_cliches: p.avoid_cliches,
    }
}
