//! Roadmap group (6), step 3: what a diff restore removes (ARCH-01 A-13h).
//!
//! `Stage::Removals` runs after `Timeline` and before `Entities`, and only
//! does anything on the diff path (after a `ClearAll` there is nothing
//! left to remove). Two domains, edges before the entities they connect:
//!
//! 1. [`RoutingRemovals`] — every aux send and sidechain key route `old`
//!    has and `new` does not keep. The removal half of `routing::Sends` /
//!    `routing::SidechainRoutes`, moved ahead of the entity removals so no
//!    edge ever names a removed bus or plugin, in the engine or in the
//!    mirror. The engine would tolerate the other order (`RemoveBus`
//!    leaves the bus's sends dangling until the app removes them, which is
//!    what the live delete does on the `BusRemoved` echo), but the mirror
//!    would name a removed endpoint in between, and the removals-first
//!    rule of A-13e (a replacement edge is cycle-checked against a graph
//!    without the edge it replaces) holds for the whole restore this way.
//! 2. [`ClipRemovals`] — every audio and MIDI clip `new` does not keep
//!    (`clips::kept_audio_clips` / `kept_midi_clips`, A-13i), before the
//!    tracks they sit on.
//! 3. [`EntityRemovals`] — every plugin instance `new` does not keep
//!    (`entities::kept_plugins`), then every track it does not keep
//!    (`entities::kept_tracks`, A-13i), then every bus `new` lacks.
//!
//! Removing before adding also lets an instance whose chain or identity
//! changed be removed and re-added under the same id: the engine refuses
//! an add whose id is still live.
//!
//! Each removal is mirrored at once, and its echo is recorded in
//! `io.restore_echoes` so the echo handlers leave the mirror alone when it
//! lands — by then a later restore may have put the same id back.

use std::collections::HashSet;

use resonance_audio::types::{AudioCommand, BusId, ClipId, PluginInstanceId, SendSource, TrackId};

use super::clips::{kept_audio_clips, kept_midi_clips};
use super::entities::{kept_plugins, kept_tracks, plugin_owners};
use super::{Reconcile, ReconcileCtx};
use crate::project::{send_source_from_tag, ProjectFile, ProjectTrack};
use crate::state::PluginLocator;
use crate::Resonance;

/// The key routes of `old` a diff restore keeps: the target plugin is kept
/// (same instance, same chain), `new` still keys it, and its source is not
/// a track the restore removes (A-13i: the engine's `RemoveTrack` drops
/// every route keyed off the track, so one onto a re-added track must be
/// set again). Every other route of `old` is cleared by
/// [`RoutingRemovals`]; `routing::SidechainRoutes` compares only these
/// against `new`, so a route onto a re-added instance is set again.
pub(super) fn kept_route_plugins(old: &ProjectFile, new: &ProjectFile) -> HashSet<PluginInstanceId> {
    let kept = kept_plugins(Some(old), new);
    let tracks = kept_tracks(Some(old), new);
    let keyed: HashSet<PluginInstanceId> =
        new.sidechain_routes.iter().map(|route| route.plugin_instance_id).collect();
    old.sidechain_routes
        .iter()
        .filter(|route| match send_source_from_tag(&route.source_kind, route.source_id) {
            Some(SendSource::Track(track_id)) => tracks.contains(&track_id),
            _ => true,
        })
        .map(|route| route.plugin_instance_id)
        .filter(|id| kept.contains(id) && keyed.contains(id))
        .collect()
}

/// The routing edges a diff restore removes, before any entity goes.
///
/// * After a `ClearAll`: nothing (`routing::Sends` / `SidechainRoutes`
///   empty their mirrors themselves).
/// * Diff: `RemoveAuxSend` for every send `old` has and `new` lacks;
///   `ClearSidechainRoute` for every route of `old` that is not kept
///   ([`kept_route_plugins`]). Each mirrored at once. Echoes
///   (`AuxSendRemoved`, `SidechainRouteChanged`) find the mirror already
///   agreeing.
pub(crate) struct RoutingRemovals;

impl Reconcile for RoutingRemovals {
    const NAME: &'static str = "routing_removals";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            return;
        };
        let target_sends: HashSet<u64> = new.sends.iter().map(|s| s.id).collect();
        for sa in &old.sends {
            if !target_sends.contains(&sa.id) {
                let _ = r.engine.send(AudioCommand::RemoveAuxSend { send_id: sa.id });
                r.aux.remove(sa.id);
            }
        }
        let kept_routes = kept_route_plugins(old, new);
        for ra in &old.sidechain_routes {
            if !kept_routes.contains(&ra.plugin_instance_id) {
                let _ = r.engine.send(AudioCommand::ClearSidechainRoute {
                    plugin: ra.plugin_instance_id,
                });
                r.sidechain.clear_plugin(ra.plugin_instance_id);
            }
        }
    }
}

/// The clips a diff restore removes (A-13i), before any track goes.
///
/// * After a `ClearAll`: nothing (the clip domains empty their mirrors).
/// * Diff: `DeleteClip` for every audio clip of `old` that is not kept
///   (`clips::kept_audio_clips` — gone from `new`, on a track the restore
///   removes, or with a different WAV or length), `DeleteMidiClip` for
///   every MIDI clip not kept; each mirrored at once, its echo owed.
///
/// Before `EntityRemovals` because `RemoveTrack` drops a track's audio
/// clips without an echo (a `DeleteClip` after it would wait on the
/// engine's load-deferral queue for a clip that never lands, and never
/// echo) and keeps its MIDI clips (which would then outlive the track).
/// This way every clip removal is one explicit command with one echo.
///
/// The mirror prune: the clip, and every piece of transient UI naming it —
/// the timeline selection, an in-flight drag, trim, fade or gain gesture,
/// the open MIDI editor or pitch editor. The pool usage, the lyric
/// side-table and the derived / vocal-audio maps are rebuilt from the
/// target by their own domains later in the restore.
pub(crate) struct ClipRemovals;

impl Reconcile for ClipRemovals {
    const NAME: &'static str = "clip_removals";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            return;
        };
        let kept = kept_audio_clips(Some(old), new);
        for oc in old.clips.iter().filter(|c| !kept.contains(&c.id)) {
            let _ = r.engine.send(AudioCommand::DeleteClip { clip_id: oc.id });
            r.io.restore_echoes.expect_clip_deleted(oc.id);
            r.clips.retain(|c| c.id != oc.id);
            prune_clip(r, oc.id);
        }
        let kept = kept_midi_clips(Some(old), new);
        for oc in old.midi_clips.iter().filter(|c| !kept.contains(&c.id)) {
            let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id: oc.id });
            r.io.restore_echoes.expect_midi_clip_deleted(oc.id);
            r.midi_clips.retain(|c| c.id != oc.id);
            prune_clip(r, oc.id);
        }
    }
}

/// Transient UI naming a clip a restore removed (audio and MIDI clips
/// share one id space).
fn prune_clip(r: &mut Resonance, clip_id: ClipId) {
    let ui = &mut r.ui.interaction;
    if ui.selected_clip == Some(clip_id) {
        ui.selected_clip = None;
    }
    if ui.selected_midi_clip == Some(clip_id) {
        ui.selected_midi_clip = None;
    }
    if ui.clip_drag.as_ref().is_some_and(|d| d.clip_id == clip_id) {
        ui.clip_drag = None;
    }
    if ui.clip_trim.as_ref().is_some_and(|d| d.clip_id == clip_id) {
        ui.clip_trim = None;
    }
    if ui.clip_fade_drag.as_ref().is_some_and(|d| d.clip_id == clip_id) {
        ui.clip_fade_drag = None;
    }
    if ui.clip_gain_drag.as_ref().is_some_and(|d| d.clip_id == clip_id) {
        ui.clip_gain_drag = None;
    }
    if ui.midi_clip_drag.as_ref().is_some_and(|d| d.clip_id == clip_id) {
        ui.midi_clip_drag = None;
    }
    if ui.midi_clip_trim.as_ref().is_some_and(|d| d.clip_id == clip_id) {
        ui.midi_clip_trim = None;
    }
    if ui.editing_midi_clip.as_ref().is_some_and(|e| e.clip_id == clip_id) {
        ui.editing_midi_clip = None;
    }
    if ui.editing_pitch_clip == Some(clip_id) {
        ui.editing_pitch_clip = None;
    }
}

/// The plugin instances, tracks and busses a diff restore removes.
///
/// * After a `ClearAll`: nothing.
/// * Diff, in `old`'s chain order: every instance `new` does not keep
///   goes (`RemovePlugin` / `RemovePluginFromBus` /
///   `RemovePluginFromMaster`) — except one on a track or bus that goes
///   too, which `RemoveTrack` / `RemoveBus` drops with it (the engine sends
///   no per-plugin echo for those). Then every track `new` does not keep
///   (`RemoveTrack`, A-13i): sub-tracks first, each by its own command, so
///   every `RemoveTrack` drops exactly its own track and chain and answers
///   with exactly one `TrackRemoved` (a parent's would also drop — and
///   echo — any sub-track still under it, but not that sub-track's chain).
///   Then every bus `new` lacks (`RemoveBus`).
///
/// A removed track's clips are already gone ([`ClipRemovals`], before this):
/// `RemoveTrack` drops its audio clips silently and keeps its MIDI clips.
/// Its edges are gone too (`RoutingRemovals`). What else names it —
/// freeze status and cache, external-instrument config, automation lanes,
/// group membership, compose tables — is reconciled against the target by
/// its own domain later in the restore.
///
/// The mirror is pruned per entity, as the echo handlers
/// (`engine_events::plugins::*_removed`, `engine_events::tracks::bus_removed`)
/// would have: the slot, its cached blob, its parked params, its
/// side-index entry, the mixer's plugin / bus selection, a key route onto
/// it or keyed off the bus. An open editor goes with its slot
/// (`PluginSlotState::editor_open`) and its instance. The missing-plugin
/// warning is derived from the chains, so a removed missing slot leaves it
/// on its own. The output-destination picker and the side-index are
/// rebuilt by `EntityOrder` at the end of `Entities`.
pub(crate) struct EntityRemovals;

impl Reconcile for EntityRemovals {
    const NAME: &'static str = "entity_removals";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            return;
        };
        let kept = kept_plugins(Some(old), new);
        let kept_tracks = kept_tracks(Some(old), new);
        let removed_tracks: Vec<&ProjectTrack> =
            old.tracks.iter().filter(|t| !kept_tracks.contains(&t.id)).collect();
        let removed_track_ids: HashSet<TrackId> = removed_tracks.iter().map(|t| t.id).collect();
        let target_busses: HashSet<BusId> = new.busses.iter().map(|b| b.id).collect();
        let removed_busses: Vec<BusId> = old
            .busses
            .iter()
            .map(|b| b.id)
            .filter(|id| !target_busses.contains(id))
            .collect();

        // `old`'s chain order, so the command sequence is deterministic.
        let owners = plugin_owners(old);
        let chain_order = old
            .tracks
            .iter()
            .flat_map(|t| t.plugins.iter())
            .chain(old.busses.iter().flat_map(|b| b.plugins.iter()))
            .chain(old.master_plugins.iter())
            .map(|pp| pp.instance_id);
        let mut removed_plugins: Vec<(PluginInstanceId, PluginLocator)> = Vec::new();
        for id in chain_order {
            if kept.contains(&id) {
                continue;
            }
            if let Some(&(owner, _)) = owners.get(&id) {
                removed_plugins.push((id, owner));
            }
        }

        for (instance_id, owner) in removed_plugins {
            let command = match owner {
                PluginLocator::Bus(bus_id) if removed_busses.contains(&bus_id) => None,
                PluginLocator::Track(track_id) if removed_track_ids.contains(&track_id) => None,
                PluginLocator::Track(track_id) => Some(AudioCommand::RemovePlugin {
                    track_id,
                    instance_id,
                }),
                PluginLocator::Bus(bus_id) => Some(AudioCommand::RemovePluginFromBus {
                    bus_id,
                    instance_id,
                }),
                PluginLocator::Master => Some(AudioCommand::RemovePluginFromMaster { instance_id }),
            };
            if let Some(command) = command {
                let _ = r.engine.send(command);
                r.io.restore_echoes.expect_plugin_removed(instance_id);
            }
            prune_plugin(r, owner, instance_id);
        }

        let (subs, others): (Vec<&ProjectTrack>, Vec<&ProjectTrack>) =
            removed_tracks.into_iter().partition(|t| t.sub_track.is_some());
        for pt in subs.into_iter().chain(others) {
            let _ = r.engine.send(AudioCommand::RemoveTrack { track_id: pt.id });
            r.io.restore_echoes.expect_track_removed(pt.id);
            prune_track(r, pt.id);
        }

        for bus_id in removed_busses {
            let _ = r.engine.send(AudioCommand::RemoveBus { bus_id });
            r.io.restore_echoes.expect_bus_removed(bus_id);
            prune_bus(r, bus_id);
        }
    }
}

/// What `engine_events::plugins::{track,bus,master}_removed` does to the
/// mirror for one instance.
fn prune_plugin(r: &mut Resonance, owner: PluginLocator, instance_id: PluginInstanceId) {
    let chain = match owner {
        PluginLocator::Track(id) => r
            .registry
            .tracks
            .iter_mut()
            .find(|t| t.id == id)
            .map(|t| &mut t.plugins),
        PluginLocator::Bus(id) => r
            .registry
            .busses
            .iter_mut()
            .find(|b| b.id == id)
            .map(|b| &mut b.plugins),
        PluginLocator::Master => Some(&mut r.master.plugins),
    };
    if let Some(chain) = chain {
        chain.retain(|p| p.instance_id != instance_id);
    }
    if r.ui.mixer.selected_plugin == Some(instance_id) {
        r.ui.mixer.selected_plugin = None;
    }
    r.plugin_mirror.state_cache.remove(&instance_id);
    // A re-add under this id (redo) parks its own copy; a stale one must
    // not be applied to it, nor written by the next save.
    r.presets.pending_plugin_param_overrides.remove(&instance_id);
    r.remove_plugin_index(instance_id);
    // `RoutingRemovals` cleared a route onto it in the engine already.
    r.sidechain.clear_plugin(instance_id);
}

/// What the live delete (`engine_events::tracks::removed`) does to the
/// mirror for one track, less what later domains reconcile against the
/// target: the registry entry (its chain was pruned by [`prune_plugin`]),
/// and every piece of transient UI that names it — the track selection,
/// its open context menu, preset-save prompt or membership drag, the
/// delete-track confirmation, the bounce dialog, the mixer's expanded
/// sub-tracks, the automation / take-lane expansion, the Compose
/// instrument focus (the drum roll's included) — and a control client's
/// pending name for it.
fn prune_track(r: &mut Resonance, track_id: TrackId) {
    use crate::state::MembershipDragSubject;

    r.registry.tracks.retain(|t| t.id != track_id);
    let ui = &mut r.ui.interaction;
    ui.deselect_track(track_id);
    ui.automation_expanded_tracks.remove(&track_id);
    ui.take_lane_expanded_tracks.remove(&track_id);
    if ui.track_menu.as_ref().is_some_and(|m| m.track_id == track_id) {
        ui.track_menu = None;
    }
    if ui.preset_save.as_ref().is_some_and(|p| p.track_id == track_id) {
        ui.preset_save = None;
    }
    if ui
        .membership_drag
        .as_ref()
        .is_some_and(|d| d.subject == MembershipDragSubject::Track(track_id))
    {
        ui.membership_drag = None;
    }
    r.ui.mixer.expanded_sub_track_parents.remove(&track_id);
    if r.modals.confirm_delete_track == Some(track_id) {
        r.modals.confirm_delete_track = None;
    }
    if r.modals.bounce_dialog.as_ref().is_some_and(|d| d.source_track_id == track_id) {
        r.modals.bounce_dialog = None;
    }
    if r.compose.expanded_track_id == Some(track_id) {
        r.compose.expanded_track_id = None;
    }
    if r.compose.details_track_id() == Some(track_id) {
        r.compose.selected_lane = crate::compose::SelectedLane::Chords;
    }
    r.control.pending_tracks.remove(&track_id);
}

/// What `engine_events::tracks::bus_removed` does to the mirror for one
/// bus, after its chain was pruned by [`prune_plugin`]. Its sends and the
/// key routes keyed off it are `RoutingRemovals`'; the tracks routed to it
/// take `new`'s output in `Tracks` (and `TrackOutputs` re-asserts it to
/// the engine, which moved them to the master on `RemoveBus`).
fn prune_bus(r: &mut Resonance, bus_id: BusId) {
    r.registry.busses.retain(|b| b.id != bus_id);
    if r.ui.mixer.selected_bus == Some(bus_id) {
        r.ui.mixer.selected_bus = None;
    }
    r.sidechain.drop_routes_from_source(SendSource::Bus(bus_id));
}
