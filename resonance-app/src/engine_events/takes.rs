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
//! (`TakeCompChanged` / `ActiveTakeChanged`) are emitted by todo #409's
//! engine work and do not exist on `AudioEvent` yet. Their projections
//! are already implemented on `TakeGroupState`, so wiring them up here is
//! a dispatch arm apiece once that lands.

use resonance_audio::types::TrackId;
use resonance_common::{TakeContent, TakeGroupId, TakeId, TimelineRange};

use crate::Resonance;

/// `TakeCaptured` — append the finished loop pass as a take, creating the
/// take group on the first pass.
pub(super) fn take_captured(
    r: &mut Resonance,
    group_id: TakeGroupId,
    track_id: TrackId,
    slot: TimelineRange,
    pass_index: u32,
    content: TakeContent,
) {
    r.take_groups.take_captured(
        group_id,
        take_id_for(pass_index),
        track_id,
        slot,
        pass_index,
        now_millis(),
        content,
    );
}

/// The take's identity within its group.
///
/// `TakeId` only has to be unique inside the owning `TakeGroup`, and the
/// engine already emits exactly such a key: `pass_index` is zero-based,
/// advanced once per loop seam, and never repeats within a record run
/// (`engine/transport.rs::finalize_loop_record_pass`). Deriving the id
/// from it keeps the mirror idempotent — a re-delivered pass replaces its
/// take instead of stacking a duplicate — without inventing app-side
/// counters the engine would then disagree with.
///
/// Todo #409 adds an engine-assigned `take_id` to the event so the
/// `SetTakeComp` / `SetActiveTake` commands reference the takes the engine
/// renders; when it lands, that field replaces this derivation and the
/// rest of the mirror is unchanged.
fn take_id_for(pass_index: u32) -> TakeId {
    TakeId::from(pass_index)
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
