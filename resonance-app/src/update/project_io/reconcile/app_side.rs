//! Roadmap group (1): app-side domains restored whole from the new file on
//! every origin. They ignore `old`.

use super::{Reconcile, ReconcileCtx};
use crate::project::ProjectFile;
use crate::update::project_io::replay::restore_tempo_events;
use crate::Resonance;

/// Tempo / signature events and the tempo map, sent to the engine.
pub(crate) struct TempoEvents;

impl Reconcile for TempoEvents {
    const NAME: &'static str = "tempo_events";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        restore_tempo_events(r, new);
        r.rebuild_and_send_tempo();
    }
}

/// Global chord track (epic #33): app-side metadata only, nothing to send.
/// Legacy projects carry none and come up with an empty track.
pub(crate) struct ChordTrack;

impl Reconcile for ChordTrack {
    const NAME: &'static str = "chord_track";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        r.chord_track = new.chord_track.to_chord_track();
    }
}

/// Arrangement markers.
pub(crate) struct Markers;

impl Reconcile for Markers {
    const NAME: &'static str = "markers";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        r.markers = crate::state::ArrangementMarkers::from(new.arrangement_markers.clone());
    }
}
