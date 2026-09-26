//! Roadmap group (2): domains shared by every origin whose body depends on
//! the origin or on live state an undo keeps (ARCH-01 A-13b).

use super::{Origin, Reconcile, ReconcileCtx};
use crate::project::ProjectFile;
use crate::Resonance;

/// Parameter-automation lanes (epic #14 / epic #40): the engine and the
/// app mirror reconciled to exactly the saved set by
/// [`Resonance::restore_automation_lanes`], which clears lanes that went
/// away (`ClearAll` does not touch engine automation) and re-sends the
/// ones that are new or changed. The same body on every origin. After
/// `ExternalInstruments`, so a `DeviceParam` lane arrives once the
/// engine knows the track's device bindings.
pub(crate) struct AutomationLanes;

impl Reconcile for AutomationLanes {
    const NAME: &'static str = "automation_lanes";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        r.restore_automation_lanes(&new.automation_lanes);
    }
}

/// The derived-clip map (`(section, placement, track) → ClipId`) and its
/// id counter, from `ProjectFile::derived_clips` (ARCH-01 A-6), through
/// [`Resonance::restore_derived_clips`]. After the MIDI and audio clips
/// are restored: it filters against them and reserves the counter past
/// them.
///
/// **The keep-rule depends on the origin.** On the diff path the engine
/// still holds every clip and an in-flight `MidiClipCreated` echo will
/// land, so every entry is kept; after a `ClearAll` only the replayed
/// clips exist, so an entry whose clip was not replayed is dropped.
///
/// **The counter floor is live state.** `ComposeState::load_from_project`
/// resets the counter before this runs, so the entry points carry the
/// live value in [`LiveCarry::derived_counter_floor`](super::LiveCarry)
/// and an undo never lowers it; a disk load has none.
///
/// A file without the field (saved before A-6) gets the positional
/// rebuild, which reads the tempo map — hence after `Timeline` on both
/// paths. Undo snapshots always carry the field.
pub(crate) struct DerivedClips;

impl Reconcile for DerivedClips {
    const NAME: &'static str = "derived_clips";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let echoes_in_flight = ctx.origin == Origin::UndoDiff;
        r.restore_derived_clips(new, echoes_in_flight, ctx.live.derived_counter_floor);
    }
}
