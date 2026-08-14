//! Clip projections: the audio and MIDI clips a track carries, as one
//! ordered wire list, plus the count `song.summary` reports.

use super::position::song_position;
use super::TPQ;
use crate::Resonance;
use resonance_control::methods::song::ClipView;

/// Every clip on `track_id` — audio and MIDI in one list, sorted by
/// start sample so the client reads them in playback order. The two
/// kinds are distinguished by [`ClipView::midi`], not by position, so a
/// caller never has to merge two lists itself.
pub(in crate::update::control) fn track_clip_views(
    app: &Resonance,
    track_id: resonance_audio::types::TrackId,
) -> Vec<ClipView> {
    let mut clips: Vec<ClipView> = Vec::new();
    for c in app.clips.iter().filter(|c| c.track_id == track_id) {
        let start_tick = app.tempo_map.sample_to_abs_tick(c.start_sample, app.sample_rate);
        let end_tick = app
            .tempo_map
            .sample_to_abs_tick(c.start_sample + c.duration_samples, app.sample_rate);
        clips.push(ClipView {
            id: resonance_control::ids::ClipId(c.id),
            name: (!c.name.is_empty()).then(|| c.name.clone()),
            start: song_position(app, c.start_sample),
            length_beats: (end_tick.saturating_sub(start_tick)) as f64 / TPQ,
            length_samples: c.duration_samples,
            midi: false,
        });
    }
    for m in app.midi_clips.iter().filter(|m| m.track_id == track_id) {
        let end_sample =
            app.tempo_map
                .tick_to_abs_sample(m.start_sample, m.duration_ticks, app.sample_rate);
        clips.push(ClipView {
            id: resonance_control::ids::ClipId(m.id),
            name: (!m.name.is_empty()).then(|| m.name.clone()),
            start: song_position(app, m.start_sample),
            length_beats: m.duration_ticks as f64 / TPQ,
            length_samples: end_sample.saturating_sub(m.start_sample),
            midi: true,
        });
    }
    clips.sort_by_key(|c| c.start.sample);
    clips
}

/// How many clips (audio + MIDI) sit on a track.
pub(in crate::update::control) fn clip_count(
    app: &Resonance,
    id: resonance_audio::types::TrackId,
) -> usize {
    app.clips.iter().filter(|c| c.track_id == id).count()
        + app.midi_clips.iter().filter(|m| m.track_id == id).count()
}
