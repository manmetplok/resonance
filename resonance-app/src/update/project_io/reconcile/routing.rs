//! Roadmap group (3): the routing edges between entities — aux sends and
//! sidechain key routes (ARCH-01 A-13e).
//!
//! Shaped like `clips::AudioClips`: after a `ClearAll` (`old = None`) the
//! mirror is emptied and every edge is sent; on the diff path the edges
//! are reconciled against `old` — removals first, then only the edges that
//! are new or changed. One body validates every edge it sends (both
//! endpoints must exist, the source kind must be known), on every origin.
//! On the diff path that is a no-op for any snapshot the app itself took:
//! `structurally_compatible` makes the track, bus and plugin sets of `old`
//! and `new` equal, and the live mirrors never hold an edge onto a missing
//! endpoint (deleting an endpoint drops its edges).
//!
//! `Stage::Routing` runs after every track, bus and the master chain
//! exist on both paths: the engine rejects a send naming an unregistered
//! endpoint, and a key route names a plugin instance id.

use std::collections::{HashMap, HashSet};

use resonance_audio::types::{AudioCommand, AuxSend, SendSource, SidechainRoute};

use super::{Reconcile, ReconcileCtx};
use crate::project::{send_source_from_tag, ProjectFile, ProjectSend, ProjectSidechainRoute};
use crate::Resonance;

/// Whether an edge's source is a track or bus the app has registered.
/// On the full path the registry is the replayed file's; on the diff path
/// it is the live one, whose id sets `structurally_compatible` made equal
/// to the target's.
fn source_exists(r: &Resonance, source: SendSource) -> bool {
    match source {
        SendSource::Track(id) => r.registry.tracks.iter().any(|t| t.id == id),
        SendSource::Bus(id) => r.registry.busses.iter().any(|b| b.id == id),
    }
}

/// The aux-send graph (ba doc #273) and its `r.aux` mirror.
///
/// * After a `ClearAll`: the mirror is emptied (`ClearAll` empties the
///   engine's send table without echoing an `AuxSendRemoved` per send, so
///   a load on top of another project would otherwise inherit routes into
///   busses that no longer exist), then every send goes out as an
///   `AddAuxSend` carrying its saved id, so send ids survive a reload.
/// * Diff: every send `old` has and `new` lacks is removed FIRST, so the
///   reconciliation is order-independent: upserting first would check a
///   send that replaces another edge for feedback loops against a graph
///   that still holds the edge it replaces (undo across "delete bus A->B,
///   create bus B->A" would have the new edge rejected as a loop, then the
///   old one removed, leaving the engine with neither while the mirror
///   shows the new one). Then every send that is new or changed goes out;
///   sends equal to `old`'s are left alone, so the common undo emits no
///   send traffic.
///
/// ARCH-04 D-2: a send `old` has is already live in the engine and goes
/// out as a `SetAuxSend`; any other (every one after a `ClearAll`) as an
/// `AddAuxSend`. The engine re-validates and re-clamps every route and
/// echoes `AuxSendChanged`, which overwrites the seeded entry. Seeding
/// here rather than waiting for the echo keeps the mixer right the moment
/// the restore returns, and puts the id in the mirror eagerly so a later
/// `Resonance::allocate_send_id` skips it.
///
/// A send whose endpoint is missing, or whose source kind this build does
/// not know, is dropped with a warning (the next save rewrites the file
/// without it) rather than mirrored: `AuxSendRejected` does not remove a
/// seeded entry, so a phantom would be drawn while no audio is routed, and
/// guessing a source kind wires the wrong signal into a bus.
pub(crate) struct Sends;

impl Reconcile for Sends {
    const NAME: &'static str = "sends";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let old_by_id: HashMap<u64, &ProjectSend> = match old {
            None => {
                r.aux.sends.clear();
                r.aux.last_rejection = None;
                HashMap::new()
            }
            Some(old) => {
                let target_ids: HashSet<u64> = new.sends.iter().map(|s| s.id).collect();
                for sa in &old.sends {
                    if !target_ids.contains(&sa.id) {
                        let _ = r.engine.send(AudioCommand::RemoveAuxSend { send_id: sa.id });
                        r.aux.remove(sa.id);
                    }
                }
                old.sends.iter().map(|s| (s.id, s)).collect()
            }
        };

        for ps in &new.sends {
            let before = old_by_id.get(&ps.id).copied();
            if before == Some(ps) {
                continue;
            }
            let Some(source) = send_source_from_tag(&ps.source_kind, ps.source_id) else {
                tracing::warn!(
                    "project restore: dropping send {} — unknown source kind {:?}",
                    ps.id, ps.source_kind
                );
                continue;
            };
            if !source_exists(r, source) || !r.registry.busses.iter().any(|b| b.id == ps.dest_bus) {
                tracing::warn!(
                    "project restore: dropping send {} — endpoint missing (source {:?}, dest bus {})",
                    ps.id, source, ps.dest_bus
                );
                continue;
            }
            let send = AuxSend {
                id: ps.id,
                source,
                dest: ps.dest_bus,
                level_db: ps.level_db,
                pre_fader: ps.pre_fader,
                enabled: ps.enabled,
            };
            let _ = r.engine.send(if before.is_some() {
                AudioCommand::SetAuxSend {
                    id: send.id,
                    source,
                    dest: send.dest,
                    level_db: send.level_db,
                    pre_fader: send.pre_fader,
                    enabled: send.enabled,
                }
            } else {
                AudioCommand::AddAuxSend {
                    id: send.id,
                    source,
                    dest: send.dest,
                    level_db: send.level_db,
                    pre_fader: send.pre_fader,
                    enabled: send.enabled,
                }
            });
            r.aux.upsert(send);
        }
    }
}

/// Every plugin instance id `file` carries, across track, bus and master
/// chains: the membership test for a key route's target. Read from the
/// file rather than `r.plugin_mirror.index` (which `EntityOrder` rebuilds
/// just before this stage on the full path since A-13f): the file is the
/// target on every origin, and the replayed chains hand the engine these
/// same ids as `id_hint`s.
fn plugin_instance_ids(file: &ProjectFile) -> HashSet<u64> {
    file.tracks
        .iter()
        .flat_map(|t| t.plugins.iter())
        .chain(file.busses.iter().flat_map(|b| b.plugins.iter()))
        .chain(file.master_plugins.iter())
        .map(|p| p.instance_id)
        .collect()
}

/// The sidechain key routes (ba doc #157/#159, todo #1311) and their
/// `r.sidechain` mirror, one route per keyed plugin instance.
///
/// * After a `ClearAll`: the mirror is emptied (`ClearAll` empties the
///   engine's route table without echoing a `SidechainRouteChanged` per
///   route), then every route goes out as `SetSidechainRoute` naming the
///   target plugin's saved instance id — the same id the replayed chain
///   handed the engine — so a route survives a reload without remapping.
/// * Diff: every route `old` has and `new` lacks is cleared first (same
///   order-independence rule as [`Sends`]; the stakes are lower, as a
///   route simply replaces whatever its plugin had), then every route that
///   is new or changed is set; unchanged routes are left alone.
///
/// A route whose source or target plugin is missing, or whose source kind
/// is unknown, is dropped with a warning rather than mirrored: a phantom
/// would be drawn while no key is delivered, and a guessed kind points the
/// detector at a different channel (track and bus ids are independent
/// namespaces).
pub(crate) struct SidechainRoutes;

impl Reconcile for SidechainRoutes {
    const NAME: &'static str = "sidechain_routes";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let old_by_plugin: HashMap<u64, &ProjectSidechainRoute> = match old {
            None => {
                r.sidechain.clear();
                HashMap::new()
            }
            Some(old) => {
                let target_plugins: HashSet<u64> =
                    new.sidechain_routes.iter().map(|route| route.plugin_instance_id).collect();
                for ra in &old.sidechain_routes {
                    if !target_plugins.contains(&ra.plugin_instance_id) {
                        let _ = r.engine.send(AudioCommand::ClearSidechainRoute {
                            plugin: ra.plugin_instance_id,
                        });
                        r.sidechain.clear_plugin(ra.plugin_instance_id);
                    }
                }
                old.sidechain_routes.iter().map(|route| (route.plugin_instance_id, route)).collect()
            }
        };
        if new.sidechain_routes.is_empty() {
            return;
        }

        let known_plugins = plugin_instance_ids(new);
        for pr in &new.sidechain_routes {
            if old_by_plugin.get(&pr.plugin_instance_id).copied() == Some(pr) {
                continue;
            }
            let Some(source) = send_source_from_tag(&pr.source_kind, pr.source_id) else {
                tracing::warn!(
                    "project restore: dropping sidechain route onto plugin {} — unknown source \
                     kind {:?}",
                    pr.plugin_instance_id, pr.source_kind
                );
                continue;
            };
            if !source_exists(r, source) || !known_plugins.contains(&pr.plugin_instance_id) {
                tracing::warn!(
                    "project restore: dropping sidechain route onto plugin {} — endpoint missing \
                     (source {:?})",
                    pr.plugin_instance_id, source
                );
                continue;
            }
            let _ = r.engine.send(AudioCommand::SetSidechainRoute {
                plugin: pr.plugin_instance_id,
                source,
                enabled: pr.enabled,
            });
            r.sidechain.upsert(SidechainRoute {
                plugin: pr.plugin_instance_id,
                source,
                enabled: pr.enabled,
            });
        }
    }
}
