//! Helpers shared by the offline (`bounce`) and realtime
//! (`bounce_realtime`) entry points. Both compute the same render
//! range — either the loop punch-in/out window or the source track's
//! MIDI extent — keeping the logic in one place so a tail/start tweak
//! lands on both flows.

use std::sync::Arc;

use super::SharedState;
use crate::types::*;

/// Tail rendered past the last MIDI clip end on the source track when
/// bouncing in place — captures FX / bus reverb decay so the bounce
/// sounds self-contained. Applies to both offline and realtime bounce.
///
/// It is also the one tail policy of every offline render whose range is
/// the project's own extent (code review ENG-07): the master export, the
/// stems (with their FX tail on), the freeze cache and the default mix
/// measurement all render this much past the last clip end, so a mix and
/// its stems come out the same length and a reverb / delay / release
/// decays into the file instead of being cut.
pub(crate) const BOUNCE_TAIL_SECONDS: u32 = 2;

/// [`BOUNCE_TAIL_SECONDS`] in frames at `sample_rate`.
pub(crate) fn offline_tail_frames(sample_rate: u32) -> u64 {
    sample_rate as u64 * BOUNCE_TAIL_SECONDS as u64
}

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
///
/// `pub` (not `pub(crate)`) only so `test_support` can re-export it
/// for integration tests.
pub fn midi_render_range(
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    shared: &Arc<SharedState>,
    source_track_id: TrackId,
    sample_rate: u32,
) -> Result<(SamplePos, SamplePos), &'static str> {
    // Loop wins. The loop end is taken as authoritative — no extra
    // tail — since the user explicitly drew that boundary.
    let range = shared.loop_range();
    if range.enabled {
        let (lo, hi) = (range.loop_in, range.loop_out);
        if hi > lo {
            return Ok((lo, hi));
        }
    }

    let tail_samples = sample_rate as u64 * BOUNCE_TAIL_SECONDS as u64;
    let graph = shared.graph.load();
    let midi_guard = &graph.midi_clips;
    let tm = tempo_map.load();

    let mut start: Option<u64> = None;
    let mut end: Option<u64> = None;
    for c in midi_guard.iter().filter(|c| c.track_id == source_track_id) {
        // Tempo-aware end: the renderer schedules notes via
        // tick_to_abs_sample, so a flat samples-per-tick conversion
        // would mis-size the range under tempo changes.
        let e =
            tm.tick_to_abs_sample(c.start_sample, c.visible_duration_ticks(), sample_rate);
        start = Some(start.map_or(c.start_sample, |s| s.min(c.start_sample)));
        end = Some(end.map_or(e, |prev| prev.max(e)));
    }
    match (start, end) {
        (Some(s), Some(e)) => Ok((s, e + tail_samples)),
        _ => Err("Source track has no MIDI clips to bounce"),
    }
}
