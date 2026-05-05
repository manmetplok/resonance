//! Helpers shared by the offline (`bounce`) and realtime
//! (`bounce_realtime`) entry points. Both compute the same render
//! range — either the loop punch-in/out window or the source track's
//! MIDI extent — keeping the logic in one place so a tail/start tweak
//! lands on both flows.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use parking_lot::RwLock;

use super::SharedState;
use crate::types::*;

/// Tail rendered past the last MIDI clip end on the source track when
/// bouncing in place — captures FX / bus reverb decay so the bounce
/// sounds self-contained. Applies to both offline and realtime bounce.
pub(crate) const BOUNCE_TAIL_SECONDS: u32 = 2;

/// Compute the sample range that needs to be rendered when bouncing
/// the MIDI on `source_track_id`. Range is `[start, end)`.
///
/// If a loop range is set on the transport (`loop_enabled` with a
/// non-empty `[loop_in, loop_out)`) the loop window wins — this is
/// what makes "select a punch-in/out region and bounce just that"
/// work. Otherwise we fall back to `[earliest MIDI start, latest MIDI
/// end + 2 s tail]` so a freshly-clicked bounce still captures the
/// reverb decay past the last note.
///
/// Returns `Err` if there's nothing to bounce — neither a loop nor
/// any MIDI clip on the source track.
pub(crate) fn midi_render_range(
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    tempo_map: &Arc<RwLock<TempoMap>>,
    shared: &Arc<SharedState>,
    source_track_id: TrackId,
    sample_rate: u32,
) -> Result<(SamplePos, SamplePos), &'static str> {
    // Loop wins. The loop end is taken as authoritative — no extra
    // tail — since the user explicitly drew that boundary.
    if shared.loop_enabled.load(Ordering::Relaxed) {
        let lo = shared.loop_in.load(Ordering::Relaxed);
        let hi = shared.loop_out.load(Ordering::Relaxed);
        if hi > lo {
            return Ok((lo, hi));
        }
    }

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
