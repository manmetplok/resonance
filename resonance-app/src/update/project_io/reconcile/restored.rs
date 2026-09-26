//! Roadmap group (2): domains shared by every origin whose body depends on
//! the origin or on live state an undo keeps (ARCH-01 A-13b).

use super::{Reconcile, ReconcileCtx};
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
