//! Roadmap group (1): app-side domains restored whole from the new file on
//! every origin. They ignore `old`.

use std::collections::HashMap;
use std::path::Path;

use resonance_audio::types::AudioCommand;

use super::{Origin, Reconcile, ReconcileCtx};
use crate::project::{ProjectFile, ProjectTrack};
use crate::state::TrackGroupRegistry;
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
/// bump (STATE-04), and the group-macro solo/mute cascade every restore
/// must re-derive (FU-A13a).
pub(crate) struct TrackGroups;

impl Reconcile for TrackGroups {
    const NAME: &'static str = "track_groups";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        restore_track_groups(r, new);
        sync_effective_track_macros(r, old, new, ctx);
    }
}

/// Re-derive every track's *effective* solo/mute — its own flag OR any
/// containing group's macro (`TrackGroupRegistry::effective_solo` /
/// `effective_mute`) — and send it to the engine wherever that may have
/// changed (FU-A13a, design doc §12 "found, not fixed").
///
/// Nothing else re-runs this. `Tracks` (`Stage::Entities`, ahead of this
/// domain) only ever sends a track's *own* flag, and on the diff path only
/// when that own flag changed — so a macro toggle's undo, or undoing a
/// member's own flag while a group macro holds, left the engine's solo/mute
/// stale even though the app-side registries were restored correctly. This
/// runs after both `Tracks` and this domain's own `restore_track_groups`,
/// so `r.track_groups` and the live `soloed`/`muted` flags `Tracks` just
/// wrote already reflect `new` — the effective value computed here is the
/// target's.
///
/// * After a `ClearAll` there is no live state to diff against (and
///   `Tracks`' full replay always re-adds every track with its bare own
///   flag), so every track's effective value is sent unconditionally —
///   correcting a saved group macro's cascade on disk load exactly as a
///   fresh toggle would.
/// * On the diff path, `old` names the snapshot the still-live engine was
///   built from, so the *old* effective value is computed the same way —
///   through a throwaway `TrackGroupRegistry::from_saved(&old.track_groups)`
///   — as what the engine holds now **unless `Tracks` just overwrote it**:
///   when a track's own flag changed, `Tracks`' diff arm already sent that
///   bare own value (`apply_track`, ahead of this domain), so what the
///   engine currently holds is `pt.muted`/`pt.soloed`, not the old
///   effective value. Only a track whose effective target differs from
///   *that* — the value the engine actually holds right now — gets a
///   command; this is what catches both a macro toggle's undo (own flag
///   unchanged, effective value flips) and undoing a member's own flag
///   while a group macro holds (own flag flips back, but `Tracks`' bare
///   send must be overridden because the macro never let go). A track
///   present only in `new` (A-13h/i, not yet possible on this path) is
///   skipped, as `Tracks`' own diff arm does.
fn sync_effective_track_macros(
    r: &mut Resonance,
    old: Option<&ProjectFile>,
    new: &ProjectFile,
    ctx: &ReconcileCtx<'_>,
) {
    if ctx.origin.after_clear_all() {
        for pt in &new.tracks {
            let soloed = r.track_groups.effective_solo(pt.id, pt.soloed);
            let muted = r.track_groups.effective_mute(pt.id, pt.muted);
            let _ = r.engine.send(AudioCommand::SetTrackSolo {
                track_id: pt.id,
                soloed,
            });
            let _ = r.engine.send(AudioCommand::SetTrackMute {
                track_id: pt.id,
                muted,
            });
        }
        return;
    }
    let Some(old) = old else {
        // No snapshot to diff against (untitled project, diff path) —
        // nothing was live to have gone stale.
        return;
    };
    let old_groups = TrackGroupRegistry::from_saved(&old.track_groups);
    let old_by_id: HashMap<u64, &ProjectTrack> = old.tracks.iter().map(|t| (t.id, t)).collect();
    for pt in &new.tracks {
        // Defence in depth — see `Tracks`: a track only in `new` is
        // A-13h/i, not reachable on the diff path yet.
        let Some(&ot) = old_by_id.get(&pt.id) else {
            continue;
        };
        // What the engine holds *right now*: `Tracks`' bare own-flag send
        // if the own flag changed (it ran first, in `Stage::Entities`),
        // else the old effective value it never touched.
        let engine_soloed = if ot.soloed != pt.soloed {
            pt.soloed
        } else {
            old_groups.effective_solo(pt.id, ot.soloed)
        };
        let target_soloed = r.track_groups.effective_solo(pt.id, pt.soloed);
        if engine_soloed != target_soloed {
            let _ = r.engine.send(AudioCommand::SetTrackSolo {
                track_id: pt.id,
                soloed: target_soloed,
            });
        }
        let engine_muted = if ot.muted != pt.muted {
            pt.muted
        } else {
            old_groups.effective_mute(pt.id, ot.muted)
        };
        let target_muted = r.track_groups.effective_mute(pt.id, pt.muted);
        if engine_muted != target_muted {
            let _ = r.engine.send(AudioCommand::SetTrackMute {
                track_id: pt.id,
                muted: target_muted,
            });
        }
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
