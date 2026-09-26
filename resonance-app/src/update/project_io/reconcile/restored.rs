//! Roadmap group (2): domains shared by every origin whose body depends on
//! the origin or on live state an undo keeps (ARCH-01 A-13b).

use super::{Origin, Reconcile, ReconcileCtx};
use crate::project::ProjectFile;
use crate::update::project_io::replay::{
    reconcile_references, restore_references, ReferenceMonitorSource,
};
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

/// External-instrument mode (`ProjectTrack::external_instrument`): the
/// bank/program/latency config and the selected device preset's param
/// bindings, through [`Resonance::restore_external_instruments`]. Before
/// `AutomationLanes`, so a `DeviceParam` lane lands on known bindings.
///
/// After a `ClearAll` the map is rebuilt from scratch and a track with no
/// device selected sends no `SetTrackDeviceParams`; on the diff path stale
/// tracks are cleared on the engine and every live offline flag survives.
/// The Bank Select + Program Change resend a disk load needs
/// (`ResendExternalInstrumentPatches`) is not part of this domain: it stays
/// in the `AllCleared` handler's disk-load tail, since an undo must never
/// re-fire MIDI at the synth.
///
/// This used to run per track inside `replay_track` on the full path.
/// Keyed by the file's track id: a legacy sub-track whose id collided and
/// was remapped in `replay_track` is never external (sub-tracks cannot be
/// made external).
pub(crate) struct ExternalInstruments;

impl Reconcile for ExternalInstruments {
    const NAME: &'static str = "external_instruments";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        r.restore_external_instruments(new, ctx.origin.after_clear_all());
    }
}

/// The reference A/B block (ARCH-01 A-5): content (entries, selection,
/// loudness match, trim) from `ProjectFile::references` /
/// `reference_settings`; the monitor state (A/B source, loop-to-mix,
/// meters) is not undo state.
///
/// **Three bodies.** After a `ClearAll` the engine's references and its id
/// allocator are gone, so [`restore_references`] re-registers every entry
/// and re-sends the whole `ReferencePlayer` state: a disk load takes the
/// monitor from the file, a full-replay undo keeps the live one (still in
/// `r.reference.monitor` — nothing in the replay touches it before this).
/// On the diff path the engine still holds every reference, so
/// [`reconcile_references`] matches by path, keeps ids and analysis, and
/// sends only what changed.
pub(crate) struct References;

impl Reconcile for References {
    const NAME: &'static str = "references";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        match ctx.origin {
            Origin::DiskLoad => restore_references(r, new, ReferenceMonitorSource::File),
            Origin::UndoFull => restore_references(r, new, ReferenceMonitorSource::Live),
            Origin::UndoDiff => reconcile_references(r, new),
        }
    }
}

/// The missing-plugin warning (ba doc #275 P5, todo #1309). App-side only;
/// nothing is read from the file.
///
/// A disk load starts with a clean slate: every slot is re-added
/// optimistically and the engine's refusals (`PluginAdded` never arrives,
/// an error does — handled later, as engine events) raise the warning
/// again for *this* project. A full-replay undo re-adds the same plugins
/// and gets the same refusals, so it dismisses the warning for the
/// project instead of re-raising the modal on every history step — the
/// same reason the missing-FILE modal is opened only for disk loads. The
/// diff path re-adds nothing, so it leaves the warning as it is.
///
/// Runs in the Tail rather than before the plugins are re-added: the
/// refusals are asynchronous engine events handled after the restore
/// returns, and nothing in the restore reads or raises the warning.
pub(crate) struct MissingPlugins;

impl Reconcile for MissingPlugins {
    const NAME: &'static str = "missing_plugins";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, _: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        match ctx.origin {
            Origin::DiskLoad => r.missing_plugins.reset(),
            Origin::UndoFull => r.missing_plugins.dismiss(),
            Origin::UndoDiff => {}
        }
    }
}

/// Track freeze status, from `ProjectTrack::freeze` (ARCH-01 A-4). Last in
/// the table: a disk load's content baseline fingerprints the replayed
/// content, automation lanes included.
///
/// **Disk load** drops the previous project's statuses, batch and UPD-05
/// baselines, then [`rehydrate_frozen_tracks`] re-attaches each frozen
/// track's cache to the engine (`SetTrackFrozenSource`, ba todo #577)
/// against `ctx.project_dir`, or loads it `Stale` when the cache is gone.
///
/// **Undo/redo, both paths**, reconciles against the live statuses through
/// [`apply_freeze_restore`]: the cache of a freeze the restore undoes is
/// detached and deleted, a restored `Frozen` whose cache is gone becomes
/// `Stale`, and the UPD-05 baselines survive (FU-H2b). The statuses are
/// live state an undo keeps; they stay in `r.freeze` (nothing in either
/// restore touches them before this runs), and the caches live beside the
/// live project path, carried in `ctx.live` because the full replay has
/// taken `io.project_path` by now.
///
/// **FU-A4a (open, not fixed here):** after a `ClearAll`, a track an undo
/// restores `Frozen` never gets `SetTrackFrozenSource` — it shows Frozen
/// and plays the live chain; the diff path has the same gap for a redo of a
/// freeze whose cache exists. The fix belongs in the undo arm below: for
/// each track `apply_freeze_restore` leaves `Frozen` (all of them after a
/// `ClearAll`; on the diff path, those that were not frozen before), decode
/// and attach the cache as `rehydrate_frozen_tracks` does, downgrading an
/// undecodable one to `Stale`.
///
/// [`rehydrate_frozen_tracks`]: Resonance::rehydrate_frozen_tracks
/// [`apply_freeze_restore`]: Resonance::apply_freeze_restore
pub(crate) struct Freeze;

impl Reconcile for Freeze {
    const NAME: &'static str = "freeze";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        if ctx.origin.is_undo() {
            r.apply_freeze_restore(&new.tracks, ctx.live.project_path);
            return;
        }
        r.freeze.reset();
        if let Some(project_dir) = ctx.project_dir {
            let freezes: Vec<_> = new
                .tracks
                .iter()
                .filter(|t| t.freeze.is_frozen)
                .map(|t| (t.id, t.freeze.clone()))
                .collect();
            r.rehydrate_frozen_tracks(project_dir, &freezes);
        }
    }
}
