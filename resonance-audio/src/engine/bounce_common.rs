//! Helpers shared by the offline (`bounce`) and realtime
//! (`bounce_realtime`) entry points. Both compute the same MIDI-driven
//! render range over a single source track plus a fixed tail; keeping
//! that logic in one place ensures any tail/start tweak applies to both
//! flows.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::types::*;

/// Tail rendered past the last MIDI clip end on the source track when
/// bouncing in place — captures FX / bus reverb decay so the bounce
/// sounds self-contained. Applies to both offline and realtime bounce.
pub(crate) const BOUNCE_TAIL_SECONDS: u32 = 2;

/// Compute the sample range that needs to be rendered when bouncing the
/// MIDI on `source_track_id`. Range is `[start, end)` and includes a
/// fixed tail past the last MIDI end. Returns `Err` if the source has
/// no MIDI clips.
pub(crate) fn midi_render_range(
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    tempo_map: &Arc<RwLock<TempoMap>>,
    source_track_id: TrackId,
    sample_rate: u32,
) -> Result<(SamplePos, SamplePos), &'static str> {
    let tail_samples = sample_rate as u64 * BOUNCE_TAIL_SECONDS as u64;
    let midi_guard = midi_clips.read();
    let tm = tempo_map.read();
    let spt = tm.samples_per_beat(sample_rate) / TICKS_PER_QUARTER_NOTE as f64;

    let mut start: Option<u64> = None;
    let mut end: Option<u64> = None;
    for c in midi_guard.iter().filter(|c| c.track_id == source_track_id) {
        start = Some(start.map_or(c.start_sample, |s| s.min(c.start_sample)));
        end = Some(end.map_or(c.end_sample(spt), |e| e.max(c.end_sample(spt))));
    }
    match (start, end) {
        (Some(s), Some(e)) => Ok((s, e + tail_samples)),
        _ => Err("Source track has no MIDI clips to bounce"),
    }
}
