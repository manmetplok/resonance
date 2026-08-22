//! Take-lane engine-event mirroring (epic #15, design doc #165).
//!
//! Folds the engine's cycle-record results into the app's take groups
//! with no read-getters back into the engine: `TakeCaptured` appends a
//! take, creating the group on the first pass. The mutation logic lives
//! on [`TakeGroupState`](crate::state::TakeGroupState); this handler stays
//! thin, supplying only the take id and wall-clock capture stamp the
//! event omits.
//!
//! The comp / active-take echoes doc #165 also lists for this seam
//! (`TakeCompChanged` / `ActiveTakeChanged`) now exist on `AudioEvent`
//! (todo #409). Their projections are already implemented on
//! `TakeGroupState`, so wiring them up here is a dispatch arm plus a thin
//! handler apiece — todo #411.

use resonance_audio::types::TrackId;
use resonance_common::{TakeContent, TakeGroupId, TakeId, TimelineRange};

use crate::Resonance;

/// `TakeCaptured` — append the finished loop pass as a take, creating the
/// take group on the first pass.
#[allow(clippy::too_many_arguments)]
pub(super) fn take_captured(
    r: &mut Resonance,
    group_id: TakeGroupId,
    take_id: TakeId,
    track_id: TrackId,
    slot: TimelineRange,
    pass_index: u32,
    content: TakeContent,
) {
    r.take_groups.take_captured(
        group_id,
        take_id,
        track_id,
        slot,
        pass_index,
        now_millis(),
        content,
    );
}

/// Wall-clock capture time in unix milliseconds, or `0` if the system
/// clock is before the epoch. The `TakeCaptured` event omits a timestamp,
/// so the app stamps each take as it mirrors it.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
