//! Roadmap group (1): app-side domains restored whole from the new file on
//! every origin. They ignore `old`.

use std::path::Path;

use super::{Origin, Reconcile, ReconcileCtx};
use crate::project::ProjectFile;
use crate::update::project_io::replay::{
    replay_take_groups, restore_performance, restore_pool_assets, restore_quantize,
    restore_tempo_events, restore_track_groups,
};
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

/// Media pool (doc #175): the asset list, each flagged missing against
/// the project directory, and usage recomputed from the clips' asset refs
/// (restored earlier). After a `ClearAll` the engine's asset-id allocator
/// is also pushed past every restored id (ba doc #276 BUG 2); the diff
/// path never needed that, the allocator being monotonic in a session.
pub(crate) struct Pool;

impl Reconcile for Pool {
    const NAME: &'static str = "pool";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        restore_pool_assets(r, new, ctx.project_dir, ctx.origin.after_clear_all());
    }
}

/// MIDI quantize state (ba todo #395).
pub(crate) struct Quantize;

impl Reconcile for Quantize {
    const NAME: &'static str = "quantize";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        restore_quantize(r, new);
    }
}

/// Performance footer: tuning + capo (epic #11).
pub(crate) struct Performance;

impl Reconcile for Performance {
    const NAME: &'static str = "performance";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        restore_performance(r, new);
    }
}

/// Track groups (folder tracks, epic #36), incl. the sub-track id counter
/// bump (STATE-04).
pub(crate) struct TrackGroups;

impl Reconcile for TrackGroups {
    const NAME: &'static str = "track_groups";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, _: &ReconcileCtx<'_>) {
        restore_track_groups(r, new);
    }
}

/// Cycle-record take lanes (epic #15): clear the mirror, then
/// [`replay_take_groups`] re-seeds it from `new`, resolves each audio
/// take's WAV against the project directory, and replaces the engine's
/// store with `RestoreTakeGroups` (ba todo #1394).
///
/// **The clear depends on the origin.** After a `ClearAll` it is
/// [`clear`], which also drops the waveform peaks: the mirror is built
/// purely from `TakeCaptured` echoes and `ClearAll` emptied the engine's
/// map without echoing a removal per group, so without it loading project
/// B kept project A's take lanes, with `clip_ref`s naming WAVs in a
/// different project's `audio/` directory (todo #412). `replay_take_groups`
/// only adds, so this is the one place a load drops the old project's
/// lanes.
///
/// On the diff path it is [`clear_for_snapshot`], which keeps the peaks
/// (ba todo #1400): this runs on every history step, a fader undo
/// included, and with the tables kept `replay_take_groups` skips every
/// already-read take, so an undo costs no mmap or scan per take. Safe
/// because a recording is immutable and a cached table records the
/// `clip_ref` it came from.
///
/// No project directory (an untitled project on the diff path) joins to a
/// bare relative path that won't exist, which flags rather than hides —
/// the safe way round for takes.
///
/// [`clear`]: crate::state::TakeGroupState::clear
/// [`clear_for_snapshot`]: crate::state::TakeGroupState::clear_for_snapshot
pub(crate) struct TakeGroups;

impl Reconcile for TakeGroups {
    const NAME: &'static str = "take_groups";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        match ctx.origin {
            Origin::DiskLoad | Origin::UndoFull => r.take_groups.clear(),
            Origin::UndoDiff => r.take_groups.clear_for_snapshot(),
        }
        replay_take_groups(r, new, ctx.project_dir.unwrap_or(Path::new("")));
    }
}
