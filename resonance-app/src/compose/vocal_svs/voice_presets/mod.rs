//! Voice presets: a singer and their delivery, recalled onto a vocal lane
//! in one step (`docs/features/compose/voice-presets.md`).
//!
//! A preset sets [`VoiceSettings`] — bank, speaker, voice type, range and
//! the delivery sliders — and nothing else, so a finished part keeps its
//! lyrics, melody, generator settings and expression curves when its
//! singer is swapped. The built-ins are the voices the 41-voice blind
//! listen scored 3 or more ([`builtin`]); user presets are JSON files
//! ([`store`]).

pub mod builtin;
mod settings;
pub mod store;

pub use builtin::builtin_presets;
pub use settings::VoiceSettings;
pub use store::{VoicePresetStore, VOICE_PRESET_DIR_ENV};

use resonance_music_theory::VocalParams;

/// One voice preset, built-in or the user's.
#[derive(Debug, Clone, PartialEq)]
pub struct VoicePreset {
    /// `<bank>-<speaker>` (`tiger-mystic`, `lilia`) for a built-in,
    /// `user:<name>` for a user preset.
    pub id: String,
    pub name: String,
    pub description: String,
    /// Register (`low` / `mid` / `high`), character and mode words.
    pub tags: Vec<String>,
    /// The blind-listen score, 1–5; built-ins only.
    pub score: Option<u8>,
    pub builtin: bool,
    pub settings: VoiceSettings,
}

/// The id of the user preset named `name`.
pub fn user_id(name: &str) -> String {
    format!("user:{name}")
}

/// Every preset in picker order: the built-ins best first, then `user`
/// (already sorted by name).
pub fn all<'a>(user: &'a [VoicePreset]) -> impl Iterator<Item = &'a VoicePreset> {
    builtin_presets().iter().chain(user.iter())
}

/// The preset `wanted` names: an id, or a name (case-insensitive).
/// Built-ins win a tie.
pub fn find<'a>(user: &'a [VoicePreset], wanted: &str) -> Option<&'a VoicePreset> {
    let wanted = wanted.trim();
    all(user)
        .find(|p| p.id == wanted)
        .or_else(|| all(user).find(|p| p.name.eq_ignore_ascii_case(wanted)))
}

/// The first preset (picker order) whose settings `params` sings exactly.
pub fn matching<'a>(user: &'a [VoicePreset], params: &VocalParams) -> Option<&'a VoicePreset> {
    all(user).find(|p| p.settings.matches(params))
}

/// A user preset capturing `params`' voice and delivery as `name`.
pub fn capture(name: &str, description: String, tags: Vec<String>, params: &VocalParams) -> VoicePreset {
    let name = name.trim().to_owned();
    VoicePreset {
        id: user_id(&name),
        name,
        description,
        tags,
        score: None,
        builtin: false,
        settings: VoiceSettings::from_params(params),
    }
}
