//! Vocal render-state test hooks (vocal-mcp-control-and-quality.md §7):
//! a lane's state, its debounced GUI re-render, and a stand-in for the
//! off-thread render landing — no ONNX inference runs in a test.

use resonance_audio::types::TrackId;

use crate::compose::VocalLaneRenderStatus;
use crate::Resonance;

impl Resonance {
    /// Test-only: the lane's render state now (what `song.vocal` reports).
    #[doc(hidden)]
    pub fn test_vocal_lane_render_status(
        &self,
        definition_id: u64,
        track_id: TrackId,
    ) -> VocalLaneRenderStatus {
        crate::update::compose::vocal_render_state::lane_render_status(
            self,
            (definition_id, track_id),
        )
    }

    /// Test-only: the lane's render state as the views read it — settled
    /// after the last update.
    #[doc(hidden)]
    pub fn test_vocal_settled_render_status(
        &self,
        definition_id: u64,
        track_id: TrackId,
    ) -> Option<VocalLaneRenderStatus> {
        self.compose
            .vocal_audio
            .status
            .lanes
            .get(&(definition_id, track_id))
            .cloned()
    }

    /// Test-only: the generation of the lane's pending debounced GUI
    /// re-render (the `VocalAutoRender` message that would fire), if one
    /// is armed.
    #[doc(hidden)]
    pub fn test_vocal_auto_render_pending(
        &self,
        definition_id: u64,
        track_id: TrackId,
    ) -> Option<u64> {
        self.compose
            .vocal_audio
            .status
            .auto_render
            .get(&(definition_id, track_id))
            .copied()
    }

    /// Test-only: the unit keys the lane's current content would render.
    #[doc(hidden)]
    pub fn test_vocal_expected_unit_keys(
        &self,
        definition_id: u64,
        track_id: TrackId,
    ) -> Option<Vec<u64>> {
        crate::update::compose::vocal_render_state::expected_unit_keys(
            self,
            (definition_id, track_id),
        )
        .map(|keys| keys.to_vec())
    }

    /// Test-only: land the lane's in-flight render as the render job
    /// would — a take of the content the render was queued for (the
    /// lane's current plan) on every placement — through the real
    /// `VocalAudioReady` path. Returns `false` (and does nothing) when no
    /// render is in flight.
    #[doc(hidden)]
    pub fn test_finish_vocal_render(&mut self, definition_id: u64, track_id: TrackId) -> bool {
        self.finish_vocal_render(definition_id, track_id, None)
    }

    /// Test-only: [`Self::test_finish_vocal_render`], shaped like a real
    /// take: placed at the lane's first note (its lead-in off the lane's
    /// MIDI clip), carrying the render's silence pad at both ends with
    /// `pre_roll_frames` of the leading one kept before the note — what
    /// `render_vocal_clip` hands the install.
    #[doc(hidden)]
    pub fn test_finish_vocal_render_take(
        &mut self,
        definition_id: u64,
        track_id: TrackId,
        pre_roll_frames: u64,
    ) -> bool {
        self.finish_vocal_render(definition_id, track_id, Some(pre_roll_frames))
    }

    fn finish_vocal_render(
        &mut self,
        definition_id: u64,
        track_id: TrackId,
        take_pre_roll: Option<u64>,
    ) -> bool {
        use crate::compose::messages::VocalAudioReadyData;
        use crate::compose::ComposeMessage;
        let lane = (definition_id, track_id);
        let Some(&render_epoch) = self.compose.vocal_audio.in_flight_render.get(&lane) else {
            return false;
        };
        let (trim_start_frames, trim_end_frames, pre_roll_frames, lead_ticks) = match take_pre_roll
        {
            None => (0, 0, 0, 0),
            Some(pre_roll) => {
                let pad = crate::compose::vocal_svs::pad_frames(self.sample_rate);
                let lead = self
                    .compose
                    .derived_clips
                    .iter()
                    .find(|((d, _, t), _)| *d == definition_id && *t == track_id)
                    .and_then(|(_, id)| self.midi_clips.iter().find(|c| c.id == *id))
                    .map_or(0, |c| {
                        crate::update::compose::vocal_render_plan::lead_ticks(&c.notes)
                    });
                (pad - pre_roll, pad, pre_roll, lead)
            }
        };
        let placements = self
            .compose
            .placements
            .iter()
            .filter(|p| p.definition_id == definition_id)
            .map(|p| (p.id, self.tempo_map.bar_to_sample(p.start_bar)))
            .collect();
        let unit_keys = self
            .test_vocal_expected_unit_keys(definition_id, track_id)
            .unwrap_or_default();
        let wav_path = std::env::temp_dir()
            .join("resonance_vocal_test")
            .join(format!("vocal_test_{definition_id}_{track_id}_{render_epoch}.wav"));
        let data = VocalAudioReadyData {
            definition_id,
            track_id,
            wav_path,
            placements,
            clip_name: "vocal".to_owned(),
            trim_start_frames,
            trim_end_frames,
            pre_roll_frames,
            lead_ticks,
            render_epoch,
            bpm: crate::update::compose::section_meter(self, definition_id).bpm,
            unit_keys,
        };
        let _ = self.update(crate::message::Message::Compose(
            ComposeMessage::VocalAudioReady(Box::new(data)),
        ));
        true
    }

    /// Test-only: fail the lane's in-flight render as the render job would
    /// (`VocalAudioFailed`). Returns `false` when none is in flight.
    #[doc(hidden)]
    pub fn test_fail_vocal_render(
        &mut self,
        definition_id: u64,
        track_id: TrackId,
        kind: crate::compose::vocal_svs::VocalRenderErrorKind,
        error: &str,
    ) -> bool {
        use crate::compose::ComposeMessage;
        let lane = (definition_id, track_id);
        let Some(&render_epoch) = self.compose.vocal_audio.in_flight_render.get(&lane) else {
            return false;
        };
        let _ = self.update(crate::message::Message::Compose(
            ComposeMessage::VocalAudioFailed {
                definition_id,
                track_id,
                render_epoch,
                error: error.to_owned(),
                kind,
            },
        ));
        true
    }
}
