//! Roadmap group (2): domains shared by every origin whose body depends on
//! the origin or on live state an undo keeps (ARCH-01 A-13b).

use std::collections::HashSet;

use resonance_audio::types::{ClipId, TrackId};

use super::entities::kept_tracks;
use super::{Origin, Reconcile, ReconcileCtx};
use crate::project::ProjectFile;
use crate::update::project_io::replay::{reconcile_references, restore_references};
use crate::Resonance;

/// The tracks of `new` this restore added to the engine rather than kept
/// (`entities::kept_tracks`, ARCH-01 A-13i). Every one after a `ClearAll`.
fn fresh_tracks(old: Option<&ProjectFile>, new: &ProjectFile) -> HashSet<TrackId> {
    let kept = kept_tracks(old, new);
    new.tracks.iter().map(|t| t.id).filter(|id| !kept.contains(id)).collect()
}

/// The derived clips whose `MidiClipCreated` echo is in flight at the
/// moment `live` was built: every map entry naming a clip the mirror does
/// not hold (the test UPD-05's "echo pending" suspension applies,
/// `Resonance::revalidate_frozen_content`). `live` is the undo's `old`,
/// built from the live state just before the restore.
fn pending_derived_echoes(live: &ProjectFile) -> HashSet<ClipId> {
    let mirrored: HashSet<ClipId> = live.midi_clips.iter().map(|mc| mc.id).collect();
    live.derived_clips
        .iter()
        .flatten()
        .map(|e| e.clip_id)
        .filter(|id| !mirrored.contains(id))
        .collect()
}

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

/// The derived-clip map (`(section, placement, track) → ClipId`), from
/// `ProjectFile::derived_clips` (ARCH-01 A-6), and the raise of the app's
/// clip-id counter past everything restored (D-7b), through
/// [`Resonance::restore_derived_clips`]. After the MIDI and audio clips
/// are restored: it filters against them and raises the counter past
/// them.
///
/// **The keep-rule depends on the origin.** An entry whose clip the
/// restore mirrored is always kept. One whose clip is not mirrored is
/// kept only while its `MidiClipCreated` echo is still in flight *now*:
/// on an undo, an entry of the live map (`old`) whose clip the live
/// mirror lacks — the engine still holds that clip and the echo will
/// land (FU-H2a). Any other unmirrored entry is dropped: after a disk
/// load's `ClearAll` only the replayed clips exist, and on an undo to a
/// snapshot taken while an echo was in flight that has since landed, the
/// restore removed the clip and nothing will echo it back (FU-A13j). Left
/// in, such an entry suspends the UPD-05 freeze check on its track until
/// the next regenerate replaces it.
///
/// **The counter is never lowered** (D-7b, `EntityIds::clips`): it lives
/// outside every restored state, so no origin resets it and none needs a
/// floor carried across the restore (A-13b's `LiveCarry`, now gone).
/// This domain only raises it.
///
/// **A disk load scans the bundle.** `audio/clip_<id>.wav` files no loaded
/// clip names (a deleted vocal render a backup still references) are
/// reserved past too (FU-A6c).
///
/// A file without the field (saved before A-6) gets the positional
/// rebuild, which reads the tempo map — hence after `Timeline` on both
/// paths. Undo snapshots always carry the field.
pub(crate) struct DerivedClips;

impl Reconcile for DerivedClips {
    const NAME: &'static str = "derived_clips";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let echoes_in_flight = match (ctx.origin, old) {
            (Origin::Undo, Some(old)) => pending_derived_echoes(old),
            _ => HashSet::new(),
        };
        r.restore_derived_clips(new, &echoes_in_flight);
        // A disk load also clears the clip WAVs in the bundle that no
        // loaded clip names (FU-A6c). An undo needs no scan: the counter
        // was already raised past them by the load or Save As that pointed
        // the session at this bundle, and nothing lowers it.
        if ctx.origin == Origin::DiskLoad {
            if let Some(dir) = ctx.project_dir {
                r.seed_clip_ids_on_disk(dir);
            }
        }
    }
}

/// External-instrument mode (`ProjectTrack::external_instrument`): the
/// bank/program/latency config and the selected device preset's param
/// bindings, through [`Resonance::restore_external_instruments`]. Before
/// `AutomationLanes`, so a `DeviceParam` lane lands on known bindings.
///
/// After a disk load's `ClearAll` the map is rebuilt from scratch and a
/// track with no device selected sends no `SetTrackDeviceParams`; on an
/// undo stale
/// tracks (a removed one included) are cleared on the engine and every live
/// offline flag survives — except on a fresh track, which gets the
/// after-`ClearAll` rule on its own (A-13i).
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

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let fresh = fresh_tracks(old, new);
        r.restore_external_instruments(new, ctx.origin.after_clear_all(), &fresh);
    }
}

/// The reference A/B block (ARCH-01 A-5): content (entries, selection,
/// loudness match, trim) from `ProjectFile::references` /
/// `reference_settings`; the monitor state (A/B source, loop-to-mix,
/// meters) is not undo state.
///
/// **Two bodies.** After a disk load's `ClearAll` the engine's references
/// are gone, so [`restore_references`] re-registers every entry and
/// re-sends the whole `ReferencePlayer` state, the monitor taken from the
/// file. On an undo the engine still holds every reference, so
/// [`reconcile_references`] matches by path, keeps ids and analysis, and
/// sends only what changed; the live monitor is left alone.
pub(crate) struct References;

impl Reconcile for References {
    const NAME: &'static str = "references";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        match ctx.origin {
            Origin::DiskLoad => restore_references(r, new),
            Origin::Undo => reconcile_references(r, new),
        }
    }
}

/// The missing-plugin warning (ba doc #275 P5, todo #1309). App-side only;
/// nothing is read from the file.
///
/// A disk load starts with a clean slate: every slot is re-added
/// optimistically and the engine's refusals (`PluginAdded` never arrives,
/// an error does — handled later, as engine events) raise the warning
/// again for *this* project. An undo leaves the warning as it is: it
/// re-adds only the plugin instances the live state lacks, and a missing
/// one among them re-raises the warning through the same refusal unless
/// the user already dismissed it for this project (A-13j keeps that, the
/// default of design doc §15; the full-replay undo used to dismiss it
/// outright, since it re-added every plugin on every history step).
///
/// Runs in the Tail rather than before the plugins are re-added: the
/// refusals are asynchronous engine events handled after the restore
/// returns, and nothing in the restore reads or raises the warning.
pub(crate) struct MissingPlugins;

impl Reconcile for MissingPlugins {
    const NAME: &'static str = "missing_plugins";

    fn reconcile(r: &mut Resonance, _: Option<&ProjectFile>, _: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        if ctx.origin == Origin::DiskLoad {
            r.missing_plugins.reset();
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
/// **Undo/redo** reconciles against the live statuses through
/// [`apply_freeze_restore`]: the cache of a freeze the restore undoes is
/// detached and deleted, a restored `Frozen` whose cache is gone becomes
/// `Stale`, and the UPD-05 baselines survive (FU-H2b). The statuses are
/// live state an undo keeps; they stay in `r.freeze` (nothing in either
/// restore touches them before this runs), and the caches live beside the
/// live project path (`ctx.project_dir` on an undo).
///
/// It also reconciles the engine's frozen sources (FU-A4a): each track the
/// target has frozen whose source the engine does not hold — those that
/// were not frozen before, and each fresh track (A-13i: added by this
/// restore, so it holds no source) — has its cache decoded and attached as
/// `rehydrate_frozen_tracks` does, an undecodable one going `Stale`. A
/// removed frozen track's cache is detached and deleted, as the live
/// delete does.
///
/// [`rehydrate_frozen_tracks`]: Resonance::rehydrate_frozen_tracks
/// [`apply_freeze_restore`]: Resonance::apply_freeze_restore
pub(crate) struct Freeze;

impl Reconcile for Freeze {
    const NAME: &'static str = "freeze";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        if ctx.origin.is_undo() {
            r.apply_freeze_restore(&new.tracks, ctx.project_dir, &fresh_tracks(old, new));
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
