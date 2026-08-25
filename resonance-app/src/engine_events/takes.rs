//! Take-lane engine-event mirroring (epic #15, design doc #165).
//!
//! Folds the engine's cycle-record results into the app's take groups
//! with no read-getters back into the engine: `TakeCaptured` appends a
//! take, creating the group on the first pass. The mutation logic lives
//! on [`TakeGroupState`](crate::state::TakeGroupState); this handler stays
//! thin, supplying only the take id and wall-clock capture stamp the
//! event omits.
//!
//! The comp / active-take echoes doc #165 lists for this seam
//! (`TakeCompChanged` / `ActiveTakeChanged`, todo #409's variants) are
//! routed here too. They are *confirmations*, not requests: the engine
//! sends one only after applying the change to what it plays and bounces,
//! so the mirror adopts them verbatim. Applying an echo that merely
//! repeats an update handler's optimistic write is a no-op, and an echo
//! for a group the app has not mirrored is ignored — the capture that
//! creates a group always precedes any comp change for it.
//!
//! Note the asymmetry the app must live with: a `SetActiveTake` naming a
//! take the engine's group does not hold is dropped with **no** echo at
//! all (ba doc #292). Silence is therefore not confirmation, which is why
//! `update::takes` validates a selection before sending it rather than
//! waiting to be told.

use resonance_audio::types::TrackId;
use resonance_common::{CompSegment, Take, TakeContent, TakeGroupId, TakeId, TimelineRange};

use crate::Resonance;

/// `TakeCaptured` — append the finished loop pass as a take, creating the
/// take group on the first pass.
///
/// `extent` is carried straight onto the [`Take`], not re-derived: it is
/// what the pass really recorded over (todo #1396), and the app has no
/// second source for it — a take clip never enters `Resonance::clips`, on
/// this path or on the project-load one.
#[allow(clippy::too_many_arguments)]
pub(super) fn take_captured(
    r: &mut Resonance,
    group_id: TakeGroupId,
    take_id: TakeId,
    track_id: TrackId,
    slot: TimelineRange,
    pass_index: u32,
    extent: TimelineRange,
    content: TakeContent,
) {
    let take = Take::new(take_id, pass_index, now_millis(), extent, content);
    r.take_groups.take_captured(group_id, track_id, slot, take);
}

/// `TakeCompChanged` — adopt the comp the engine now plays and bounces,
/// replacing the group's segments wholesale.
pub(super) fn comp_changed(r: &mut Resonance, group_id: TakeGroupId, segments: Vec<CompSegment>) {
    r.take_groups.comp_changed(group_id, segments);
}

/// `ActiveTakeChanged` — adopt (or clear) the take the engine is soloing
/// across the group's slot.
pub(super) fn active_take_changed(
    r: &mut Resonance,
    group_id: TakeGroupId,
    take_id: Option<TakeId>,
) {
    r.take_groups.active_take_changed(group_id, take_id);
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
