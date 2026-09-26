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
/// take group on the first pass, and read the pass's waveform off disk.
///
/// `extent` is carried straight onto the [`Take`], not re-derived: it is
/// what the pass really recorded over (todo #1396), and the app has no
/// second source for it — a take clip never enters `Resonance::clips`, on
/// this path or on the project-load one.
///
/// The peaks are read from the WAV the engine has just finished writing
/// (todo #1400): `close_pass_writer` finalizes the file at the loop seam
/// *before* this event is emitted, so by the time the app folds it the
/// recording is on disk and complete. Reading it here rather than being
/// told is what makes this path and the project-load one identical — see
/// [`crate::project::take_audio`] for the argument.
///
/// **A failed read does not flag the take.** Todo #412's `missing_takes`
/// means "the recording this project references is not on this machine",
/// which is a question about a project you are *opening*; a pass you just
/// recorded has the engine's word that it exists, and the engine will play
/// it from its own mapped copy whatever the app manages to read. Shouting
/// `media missing` at the end of a good take is precisely the bug #1400
/// exists to remove, so the honest degradation here is a card with no
/// waveform in it — plus a line on stderr, because a take whose file the
/// app cannot open the instant it was written is worth knowing about.
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
    let clip_ref = match content {
        TakeContent::Audio { clip_ref } => Some(clip_ref),
        TakeContent::Midi { .. } => None,
    };
    // A take is an undoable edit; snapshot before it lands (STATE-02).
    r.record_recording_edit();
    let take = Take::new(take_id, pass_index, now_millis(), extent, content);
    r.take_groups.take_captured(group_id, track_id, slot, take);

    // Only an audio take has a recording to read, and only a project with
    // a directory has somewhere to read it from — recording streams into
    // that directory, so in the running app it is always set here.
    let (Some(clip_ref), Some(project_dir)) = (clip_ref, r.io.project_path.clone()) else {
        return;
    };
    match crate::project::load_take_peaks(&project_dir, clip_ref) {
        Ok(peaks) => r
            .take_groups
            .set_peaks(group_id, take_id, clip_ref, peaks),
        Err(reason) => tracing::warn!(
            "take capture: take {take_id} of group {group_id} was recorded but its \
             audio could not be read back ({reason}) — the lane draws the card \
             without a waveform"
        ),
    }
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

/// `TakeRemoved` — drop the take the engine has removed, re-covering the
/// slot exactly as the engine did: `TakeGroupState::remove_take` runs the
/// shared `TakeGroup::remove_take` (ba todo #1401). The `TakeCompChanged` /
/// `ActiveTakeChanged` echoes right behind this one then re-apply the same
/// values, so there is nothing left for this handler to do.
///
/// Idempotent, like the other echoes: `update::takes` removes the take from
/// the mirror optimistically when it sends the command, so this normally
/// confirms a removal that already happened.
pub(super) fn take_removed(r: &mut Resonance, group_id: TakeGroupId, take_id: TakeId) {
    r.take_groups.remove_take(group_id, take_id);
}

/// `TakeGroupRemoved` — drop the whole lane. Sent for a `RemoveTakeGroup`
/// and for the `RemoveTake` that took a group's last take with it, which is
/// the case the app cannot infer: it asked to remove one take and the lane
/// went with it (ba todo #1397).
pub(super) fn take_group_removed(r: &mut Resonance, group_id: TakeGroupId) {
    r.take_groups.remove_group(group_id);
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
