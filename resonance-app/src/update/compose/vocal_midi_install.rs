//! Installing a freshly-derived vocal MIDI clip at every placement of a
//! section. Split out of `vocal_render` (ba todo #1259): the MIDI side
//! is installed synchronously and is independent of the off-thread audio
//! render that follows it.

use resonance_audio::types::TrackId;

/// Bundled inputs for installing a freshly-derived vocal MIDI clip
/// across every placement of a definition. Replaces the prior 8-arg
/// `install_vocal_midi` function — too many bare parallel arguments
/// hid a real mixed-responsibility problem.
pub(super) struct VocalMidiInstall<'a> {
    pub definition_id: u64,
    pub track_id: TrackId,
    pub placements: &'a [(u64, u64)],
    pub duration_ticks: u64,
    pub midi_notes: &'a [resonance_audio::types::MidiNote],
    pub lyrics: &'a [String],
    pub name: &'a str,
}

impl VocalMidiInstall<'_> {
    pub(super) fn install(&self, r: &mut crate::Resonance) {
        use resonance_audio::types::AudioCommand;
        for &(placement_id, start_sample) in self.placements {
            if let Some(old_id) =
                r.compose
                    .derived_clips
                    .remove(&(self.definition_id, placement_id, self.track_id))
            {
                crate::engine_events::midi::send_mirrored_delete(r, old_id);
                r.compose.vocal_audio.clip_lyrics.remove(&old_id);
                r.midi_clips.retain(|c| c.id != old_id);
            }
            let clip_id = r.compose.fresh_derived_clip_id();
            let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
                clip_id,
                track_id: self.track_id,
                start_sample,
                duration_ticks: self.duration_ticks,
                notes: self.midi_notes.to_vec(),
                name: self.name.to_string(),
                trim_start_ticks: 0,
                trim_end_ticks: 0,
            });
            r.compose
                .derived_clips
                .insert((self.definition_id, placement_id, self.track_id), clip_id);
            // Mirror the clip into `r.midi_clips` synchronously, the way
            // `notes.create_clip` does (ba todo #1162): the control
            // endpoint's `vocal.generate` returns this clip_id and a
            // follow-up `song.notes` / `notes.*` resolves the target
            // through `r.midi_clips`, which would otherwise still be
            // racing the async `MidiClipCreated` echo. That echo's
            // `clip_created` handler skips ids already present, so the
            // round trip stays a no-op once it lands.
            if !r.midi_clips.iter().any(|c| c.id == clip_id) {
                r.midi_clips.push(crate::state::MidiClipState {
                    id: clip_id,
                    track_id: self.track_id,
                    start_sample,
                    duration_ticks: self.duration_ticks,
                    name: self.name.to_string(),
                    notes: self.midi_notes.to_vec(),
                    trim_start_ticks: 0,
                    trim_end_ticks: 0,
                });
            }
            let mut padded: Vec<String> = self.lyrics.to_vec();
            padded.resize(self.midi_notes.len(), String::new());
            r.compose.vocal_audio.clip_lyrics.insert(clip_id, padded);
        }
    }
}
