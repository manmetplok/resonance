//! Voice preset test hooks (`docs/features/compose/voice-presets.md`):
//! a lane's full params, the user preset folder and the inspector's
//! prompt.

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::vocal_svs::voice_presets::VoicePreset;
use crate::Resonance;

impl Resonance {
    /// Test-only: a vocal lane's full params (draft, curves and all).
    #[doc(hidden)]
    pub fn test_vocal_lane_params(
        &self,
        definition_id: u64,
        track_id: TrackId,
    ) -> Option<VocalParams> {
        crate::update::voice_presets::lane_params(self, definition_id, track_id)
    }

    /// Test-only: overwrite a vocal lane's params directly (no undo entry),
    /// to give it settings no single surface sets in one step (curves,
    /// a custom delivery) before an apply.
    #[doc(hidden)]
    pub fn test_set_vocal_lane_params(
        &mut self,
        definition_id: u64,
        track_id: TrackId,
        params: VocalParams,
    ) {
        use crate::compose::LaneGeneratorKind;
        if let Some(cfg) = self
            .compose
            .find_definition_mut(definition_id)
            .and_then(|d| d.lane_generators.get_mut(&track_id))
        {
            cfg.kind = LaneGeneratorKind::Vocal(params);
        }
    }

    /// Test-only: the user voice presets as the app holds them.
    #[doc(hidden)]
    pub fn test_user_voice_presets(&self) -> Vec<VoicePreset> {
        self.voice_presets.user.clone()
    }

    /// Test-only: this app's user voice preset folder (a private folder in
    /// a test app).
    #[doc(hidden)]
    pub fn test_voice_preset_dir(&self) -> Option<std::path::PathBuf> {
        self.voice_presets
            .store
            .dir()
            .map(std::path::Path::to_path_buf)
    }

    /// Test-only: the open voice preset prompt's name, if any.
    #[doc(hidden)]
    pub fn test_voice_preset_prompt(&self) -> Option<String> {
        self.voice_presets.prompt.as_ref().map(|p| p.name.clone())
    }

    /// Test-only: the last voice preset save / rename / delete failure.
    #[doc(hidden)]
    pub fn test_voice_preset_error(&self) -> Option<String> {
        self.voice_presets.error.clone()
    }

    /// Test-only: the Voice preset picker's cached options as
    /// `(id, label)`, in picker order.
    #[doc(hidden)]
    pub fn test_voice_preset_choices(&self) -> Vec<(String, String)> {
        self.ui
            .view_caches
            .voice_preset_choices
            .iter()
            .map(|c| (c.id.to_string(), c.label.to_string()))
            .collect()
    }
}
