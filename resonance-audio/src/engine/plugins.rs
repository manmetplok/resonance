//! Track-plugin handlers: add/remove instances, parameter writes,
//! GUI open/close, individual + bulk state save/load. Any handler that
//! touches a plugin instance must `try_lock` — if the audio callback
//! holds the lock, the command gets re-enqueued via `cmd_tx_retry` so
//! the audio thread is never blocked.

use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

use crate::clap_host::{ClapBundle, ClapBundleError, ParamsRefresh, PresetHostReport};
use crate::types::*;

use super::external_instrument::ExternalInstruments;
use super::thread::{HandlerCtx, HandlerState};

/// Failure resolving or loading the `.clap` bundle behind a plugin add
/// ([`ensure_bundle`] / [`resolve_plugin_id`]). Message text matches the
/// historical `format!()` / literal strings.
#[derive(Debug, Error)]
pub enum PluginBundleError {
    #[error("Failed to load plugin: {0}")]
    Load(#[from] ClapBundleError),
    #[error("No plugins found in file")]
    NoPluginsFound,
}

impl From<PluginBundleError> for EngineError {
    fn from(e: PluginBundleError) -> Self {
        EngineError::new(EngineErrorKind::Plugin, e.to_string())
    }
}

/// True for commands that can change a track's or bus's chain latency
/// (plugin add/remove, routing, track/bus topology, freeze / FX-bypass
/// state). The engine loop republishes the plugin-delay-compensation
/// table after these run. `pub` (via `test_support`) so integration
/// tests can pin the command set.
pub fn affects_latency(cmd: &AudioCommand) -> bool {
    matches!(
        cmd,
        AudioCommand::AddPlugin { .. }
            | AudioCommand::RemovePlugin { .. }
            // Reordering looks latency-neutral — a chain's latency is the
            // sum of its plugins — but it isn't. On an instrument track the
            // *first* plugin is the instrument: it keeps running while the
            // FX chain is bypassed, and every sub-track inherits its
            // latency (see `latency::chain_latencies`). Moving a plugin
            // into or out of slot 0 therefore changes the comp table.
            | AudioCommand::MovePlugin { .. }
            | AudioCommand::AddPluginToBus { .. }
            | AudioCommand::RemovePluginFromBus { .. }
            // Bus chains have no structural slot 0, but the comp
            // table is rebuilt per chain, and rebuilding it after a
            // no-op reorder is cheap next to getting it wrong.
            | AudioCommand::MovePluginInBus { .. }
            | AudioCommand::ScanPlugins
            | AudioCommand::SetTrackOutput { .. }
            | AudioCommand::AddTrack { .. }
            | AudioCommand::AddInstrumentTrack { .. }
            | AudioCommand::AddVocalTrack { .. }
            | AudioCommand::CreateSubTrack { .. }
            | AudioCommand::RemoveTrack { .. }
            | AudioCommand::AddBus { .. }
            | AudioCommand::RemoveBus { .. }
            | AudioCommand::ClearAll
            // External-instrument config carries a manual round-trip
            // latency offset folded into the comp table; setting,
            // clearing or re-dialling it changes per-track compensation.
            | AudioCommand::SetExternalInstrument { .. }
            | AudioCommand::ClearExternalInstrument { .. }
            | AudioCommand::SetExternalInstrumentLatencyOffset { .. }
            // Freeze and FX bypass change which plugins actually run:
            // frozen tracks play a pre-trimmed cache and bypassed chains
            // are skipped, so their latency must leave the comp table
            // (see `latency::chain_latencies`).
            | AudioCommand::SetTrackFrozenSource { .. }
            | AudioCommand::UnfreezeTrack { .. }
            // Loading plugin state cycles the instance's activation and
            // re-reads its latency (doc #260 finding #10) — a preset
            // that implies a different latency (e.g. a longer IR) must
            // land in the comp table.
            | AudioCommand::LoadPluginState { .. }
            | AudioCommand::LoadPluginPresetState { .. }
            | AudioCommand::LoadPluginPresetFromLocation { .. }
            | AudioCommand::SetTrackFxBypass { .. }
            | AudioCommand::SetBusFxBypass { .. }
            // Per-slot bypass: a host-bypassed slot stops running and its
            // latency leaves its chain (`latency::slot_latency`). A slot
            // that bypasses itself through its own parameter keeps its
            // latency, and the recompute is then a cheap no-op that
            // `delays_match` drops before any delay line is reset.
            | AudioCommand::SetPluginBypass { .. }
            // Master-chain edits don't change per-track comp (master
            // delays every path equally) but they feed the published
            // master-latency figure the reference A/B monitor is
            // aligned with (doc #260 finding #19).
            | AudioCommand::AddPluginToMaster { .. }
            | AudioCommand::RemovePluginFromMaster { .. }
            // Reordering the master chain leaves its total latency
            // unchanged, but rebuilding the figure after a no-op reorder
            // is cheap next to publishing a stale one.
            | AudioCommand::MovePluginInMaster { .. }
            | AudioCommand::SetMasterFxBypass { .. }
    )
}

/// Recompute per-track compensation delays from the current topology
/// and publish a fresh table for the audio callback. Skips the publish
/// (and thus the delay-line reset) when no delay actually changed.
/// Runs on the engine thread; delay lines are allocated here, never on
/// the audio callback.
pub(crate) fn refresh_latency_comp(ctx: &HandlerCtx, external: &ExternalInstruments) {
    let (mut chains, bus_chains, master_latency) = {
        // One graph for tracks, busses, master and plugins, so the table
        // is built from a single consistent topology.
        let graph = ctx.shared.graph.load();
        let plugins_guard = &graph.plugins;
        let latency_of = |id: crate::types::PluginInstanceId| {
            plugins_guard
                .get(&id)
                .map(|slot| {
                    // A host-bypassed slot is never processed, so it adds
                    // no latency — and its instance need not be locked to
                    // find that out (ba doc #275 finding X3).
                    let host_bypassed = slot.host_bypassed();
                    let reported = (!host_bypassed)
                        .then(|| super::try_lock_with_backoff(slot).0.latency_samples() as u64)
                        .unwrap_or(0);
                    crate::latency::slot_latency(reported, host_bypassed)
                })
                .unwrap_or(0)
        };
        (
            crate::latency::chain_latencies(&graph.tracks, latency_of),
            crate::latency::bus_chain_latencies(&graph.busses, latency_of),
            crate::latency::master_chain_latency(
                &graph.master.plugin_ids,
                ctx.shared.master_fx_bypass.bypassed(),
                latency_of,
            ),
        )
    };
    // Master latency isn't compensated per-track (it delays every path
    // equally) but the reference A/B monitor aligns against it
    // (doc #260 finding #19).
    ctx.shared
        .master_latency_samples
        .store(master_latency, std::sync::atomic::Ordering::Relaxed);
    // External-instrument tracks add a manual round-trip latency offset on
    // top of their plugin chain so the rest of the mix is delayed to align
    // with the late hardware audio return.
    crate::latency::add_external_offsets(&mut chains, |id| {
        external
            .get(&id)
            .map(|c| c.latency_offset_samples)
            .unwrap_or(0)
    });
    // Publish the offsets snapshot for the offline bounce/export
    // threads, which build their own comp tables off the engine thread
    // and must fold the same offsets (doc #260 finding #4). Refreshed
    // here because every offsets change routes through this function
    // (affects_latency covers the external-instrument commands and the
    // ping applies its measurement via refresh too).
    let offsets: std::collections::HashMap<crate::types::TrackId, i64> = external
        .iter()
        .map(|(&id, c)| (id, c.latency_offset_samples))
        .collect();
    super::retire::publish(
        &ctx.shared.external_offsets,
        Arc::new(offsets),
        &ctx.shared.retired,
    );
    // Surface the MAX_COMP_LATENCY clamp: beyond it, compensation
    // silently stops matching the real chain latency and alignment
    // degrades. Warn once per engagement (and re-arm when the chains
    // drop back under the limit) via the engine's normal error surface
    // (doc #260 finding #20).
    let clamped = crate::latency::comp_latency_clamped(&chains)
        || crate::latency::comp_latency_clamped(&bus_chains);
    let was_engaged = ctx
        .shared
        .comp_clamp_engaged
        .swap(clamped, std::sync::atomic::Ordering::Relaxed);
    if clamped && !was_engaged {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::plugin(format!(
            "Plugin-delay compensation limit reached: a chain reports more than {} samples \
             of latency; timing for that path is no longer fully compensated. Consider \
             bypassing or removing the highest-latency plugin.",
            crate::limits::MAX_COMP_LATENCY
        ))));
    }
    let (track_max, track_delays) = crate::latency::compensation_delays(&chains);
    let (bus_max, bus_delays) = crate::latency::compensation_delays(&bus_chains);
    if ctx
        .latency_comp
        .load()
        .delays_match(track_max, &track_delays, bus_max, &bus_delays)
    {
        return;
    }
    // The replaced table (delay lines of up to MAX_COMP_LATENCY floats
    // each) is retired, not dropped: the callback may still be inside a
    // block that loaded it (code review MIX-04).
    super::retire::publish(
        ctx.latency_comp,
        Arc::new(crate::latency::LatencyComp::new(
            track_max,
            &track_delays,
            bus_max,
            &bus_delays,
        )),
        &ctx.shared.retired,
    );
}

/// Service plugin-initiated host callbacks (doc #260 finding #10):
/// `clap_host_latency.changed()` and `clap_host.request_restart()` both
/// flag the instance's host data; this poll — run once per engine-loop
/// iteration — consumes the flags and performs the CLAP-sanctioned
/// deactivate → reactivate cycle, which re-reads the plugin's latency
/// (it may only change across that boundary, and the bridge serves an
/// activation-time cache while active — todo #1125). If any instance
/// cycled, the PDC table is republished so the new latency takes
/// effect.
///
/// Only an instance whose lock-free pending flag is set is locked (code
/// review HOST-02 / RT-07): the host callbacks, a queued preset report and
/// output parameter events left by `process()` all raise it
/// (`PluginSlot::take_poll_pending`). Before, every instance was locked on
/// every pass — after every command and on every tick — and the audio
/// thread, which never waits for a plugin lock, skipped whichever plugin
/// the poll happened to hold: dry audio from an effect, a silent block
/// from an instrument.
///
/// An instance whose lock is held by the audio callback keeps its flag;
/// the next iteration (~16 ms) retries.
pub(crate) fn poll_plugin_host_requests(ctx: &HandlerCtx, external: &ExternalInstruments) {
    let mut any_restarted = false;
    {
        let plugins_guard = ctx.plugins();
        for (&instance_id, slot) in plugins_guard.iter() {
            if !slot.take_poll_pending() {
                continue;
            }
            let Some(mut inst) = slot.try_lock() else {
                slot.flag_poll();
                continue;
            };
            // Deliver the plugin's requested main-thread callback first:
            // that is where a plugin reports a self-closed editor
            // (`clap_host_gui.closed()`, PLG-01) or a latency change.
            inst.0.run_requested_callback();
            // A plugin that changed a param itself asked for a flush to
            // deliver it (its transport may be stopped); then whatever it
            // reported — from that flush or from `process()` since the
            // last poll — goes to the app as edits.
            inst.0.service_flush_request();
            for edit in inst.0.take_param_edits() {
                let _ = ctx
                    .event_tx
                    .send(AudioEvent::PluginParamEdited { instance_id, edit });
            }
            // A load asked for a second look at the params, or the plugin
            // asked for a rescan (`clap_host_params.rescan`). Values only
            // when that is all it said: this runs under the lock the audio
            // thread drops a block rather than wait for, and a plugin
            // reporting a load progress asks every few percent.
            let refresh = inst.0.take_params_refresh();
            let rescanned = !matches!(refresh, ParamsRefresh::None);
            match refresh {
                ParamsRefresh::None => {}
                ParamsRefresh::Values { all_text } => {
                    let values = inst.0.refresh_param_values(all_text);
                    if !values.is_empty() {
                        let _ = ctx
                            .event_tx
                            .send(AudioEvent::PluginParamValuesChanged { instance_id, values });
                    }
                }
                ParamsRefresh::Full => {
                    let _ = ctx.event_tx.send(AudioEvent::PluginParamsRefreshed {
                        instance_id,
                        params: inst.0.query_params(),
                    });
                }
            }
            // A drum plugin's pads (`com.resonance.kit-info`): read once
            // after creation, then after each rescan it asked for — the
            // extension's contract for when they may change. Sent after
            // the values, so the app sees the kit name and the pads of
            // one kit together.
            if rescanned || inst.0.kit_info_unread() {
                if let Some(info) = inst.0.poll_kit_info() {
                    let _ = ctx
                        .event_tx
                        .send(AudioEvent::PluginKitInfo { instance_id, info });
                }
            }
            // What the plugin said about its preset, from that callback or
            // a `from_location` / state load since the last poll.
            for report in inst.0.take_preset_reports() {
                let event = match report {
                    PresetHostReport::Identity(identity) => {
                        AudioEvent::PluginPresetIdentity { instance_id, identity }
                    }
                    PresetHostReport::Loaded { location, load_key } => {
                        AudioEvent::PluginPresetLoaded { instance_id, location, load_key }
                    }
                    PresetHostReport::Error { message } => AudioEvent::Error(EngineError::plugin(
                        format!("Plugin instance {instance_id} could not load a preset: {message}"),
                    )),
                };
                let _ = ctx.event_tx.send(event);
            }
            // The user closed the editor from the floating window's own
            // titlebar and the plugin told us via `clap_host_gui.closed()`
            // (ba todo #1347). `take_gui_closed` finishes the CLAP-side
            // teardown; the event is what stops the app's slot from
            // claiming an editor that is gone.
            if inst.0.take_gui_closed() {
                let _ = ctx.event_tx.send(AudioEvent::PluginEditorState {
                    instance_id,
                    open: false,
                    failure: None,
                });
            }
            let (restarted, event) = service_host_restart_request(&mut inst.0, instance_id);
            any_restarted |= restarted;
            if let Some(event) = event {
                let _ = ctx.event_tx.send(event);
            }
        }
    }
    if any_restarted {
        refresh_latency_comp(ctx, external);
    }
}

/// Act on one instance's pending restart / latency-change request:
/// returns whether it (re)activated, plus the error to report, if any.
///
/// * active: the deactivate → reactivate cycle that re-reads latency;
/// * deactivated by an earlier failure: a latency change alone needs
///   nothing — it is read at the next activation, and treating it as a
///   failed restart was a spurious error (FU-M1b) — while a restart
///   request retries the activation (FU-F2c);
/// * a failure is reported once, not again for every later request from
///   a plugin that stays deactivated (ENG-12).
pub fn service_host_restart_request(
    inst: &mut crate::clap_host::ClapInstance,
    instance_id: PluginInstanceId,
) -> (bool, Option<AudioEvent>) {
    let (restart, latency) = inst.take_host_restart_requests();
    if !restart && !(latency && inst.is_active()) {
        return (false, None);
    }
    if inst.restart() {
        return (true, None);
    }
    if !inst.take_restart_failure_report() {
        return (false, None);
    }
    (
        false,
        Some(AudioEvent::Error(EngineError::plugin(format!(
            "Plugin instance {} failed to reactivate after a restart/latency-change \
             request; it is deactivated and will stay silent.",
            instance_id
        )))),
    )
}

/// Refuse an add whose id is already live in `ctx.plugins()`, rather than
/// silently replacing the instance it names — shared by the track, bus
/// and master add paths (ARCH-04 D-1).
///
/// Every add now carries an app-allocated id
/// (`Resonance::allocate_plugin_id`); the engine has no allocator of its
/// own left to fall back on, so a collision here is not a legitimate
/// retry to smooth over — it means the app's mirror and the engine's
/// live set have drifted (a bug in the app's id bookkeeping, or the
/// GUI's own `PluginAdded` echo still in flight when the same slot is
/// re-armed). `EngineErrorKind::Internal`, not `Busy`: nothing about
/// retrying the identical command would make it succeed, which is what
/// distinguishes this from `AddBus`'s `MAX_BUSSES` refusal.
///
/// Returns `true` (and has already reported the error) when the add must
/// stop here.
pub(crate) fn reject_if_plugin_id_in_use(
    ctx: &HandlerCtx,
    id: PluginInstanceId,
    clap_plugin_id: &str,
) -> bool {
    if ctx.plugins().contains_key(&id) {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(format!(
            "plugin instance id {id} ({clap_plugin_id}) is already in use; refusing the add \
             rather than replacing the live instance"
        ))));
        true
    } else {
        false
    }
}

/// Unpublish the plugin instances `ids` from the render graph in one
/// edit (code review ARCH-02 B-4). Ids the graph does not hold are
/// ignored, and nothing is published when it holds none of them.
///
/// Nothing is destroyed here: `edit_plugins` retires each removed slot,
/// and the engine loop's sweep runs its `ClapInstance::drop` once no
/// block or offline chunk pins it — so neither this thread's caller nor
/// the audio thread ever pays for a deactivate / destroy inline.
pub(crate) fn remove_plugin_slots(shared: &super::SharedState, ids: &[PluginInstanceId]) {
    let any_live = {
        let graph = shared.graph.load();
        ids.iter().any(|id| graph.plugins.contains_key(id))
    };
    if !any_live {
        return;
    }
    shared.edit_plugins(|plugins| {
        for id in ids {
            plugins.shift_remove(id);
        }
    });
}

/// Unpublish every plugin instance and wait — sweeping the retire
/// queue — until each one has been destroyed on the calling (engine)
/// thread, or `timeout` passes. Returns whether every instance went.
///
/// For engine shutdown, where no later sweep will come: a block still
/// in flight holds the previous graph (and with it the slots) for at
/// most one callback, so this normally returns after a sweep or two.
pub(crate) fn release_all_plugins(shared: &super::SharedState, timeout: std::time::Duration) -> bool {
    let released: Vec<std::sync::Weak<crate::clap_host::PluginSlot>> =
        shared.edit_plugins(|plugins| {
            plugins
                .drain(..)
                .map(|(_, slot)| Arc::downgrade(&slot))
                .collect()
        });
    let deadline = std::time::Instant::now() + timeout;
    loop {
        shared.retired.sweep();
        if released.iter().all(|slot| slot.strong_count() == 0) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub(crate) fn handle_add_plugin(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    clap_file_path: String,
    clap_plugin_id: String,
    id: PluginInstanceId,
) {
    if reject_if_plugin_id_in_use(ctx, id, &clap_plugin_id) {
        return;
    }

    let path = Path::new(&clap_file_path);

    let bundle_idx = match ensure_bundle(&mut state.bundles, path, &clap_plugin_id) {
        Ok(idx) => idx,
        Err(reason) => {
            report_plugin_load_failure(ctx, Some(id), &clap_plugin_id, &clap_file_path, reason.to_string());
            return;
        }
    };

    let actual_plugin_id = match resolve_plugin_id(&state.bundles[bundle_idx], clap_plugin_id.clone())
    {
        Ok(resolved) => resolved,
        Err(reason) => {
            report_plugin_load_failure(ctx, Some(id), &clap_plugin_id, &clap_file_path, reason.to_string());
            return;
        }
    };

    let plugin_name = state.bundles[bundle_idx]
        .descriptors()
        .iter()
        .find(|d| d.id == actual_plugin_id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| actual_plugin_id.clone());

    match state.bundles[bundle_idx].create_instance(&actual_plugin_id, ctx.sample_rate) {
        Ok(instance) => {
            let instance_id = id;

            // Query params + has_gui + output port layout before moving
            // instance into shared map.
            let params = instance.query_params();
            let has_gui = instance.has_gui();
            let has_sidechain_input = instance.has_sidechain_input();
            let output_port_count = instance.output_port_count();
            let output_port_names = instance.output_port_names();

            // Publish the slot first, then name it on the chain: a block
            // between the two sees the slot unused, never a chain id with
            // no instance behind it.
            let slot = Arc::new(crate::clap_host::PluginSlot::new(instance));
            ctx.shared.edit_plugins(|plugins| plugins.insert(instance_id, slot));

            // `push_plugin` publishes the new chain via `ArcSwap::store`
            // (shared by every copy of the track), so no render-graph
            // publish — the audio thread is not blocked by the edit.
            if let Some(track) = ctx.tracks().get(&track_id) {
                ctx.shared.retired.retire(track.push_plugin(instance_id));
            }

            let _ = ctx.event_tx.send(AudioEvent::PluginAdded {
                track_id,
                instance_id,
                plugin_name,
                clap_plugin_id: actual_plugin_id,
                clap_file_path,
                params,
                has_gui,
                has_sidechain_input,
                output_port_count,
                output_port_names,
            });
        }
        Err(e) => report_plugin_load_failure(
            ctx,
            Some(id),
            &actual_plugin_id,
            &clap_file_path,
            format!("Failed to create plugin instance: {}", e),
        ),
    }
}

pub(crate) fn handle_remove_plugin(
    ctx: &HandlerCtx,
    track_id: TrackId,
    instance_id: PluginInstanceId,
) {
    // `retain_plugins` publishes a new chain via `ArcSwap::store` (shared
    // by every copy of the track), so reading the published track map is
    // enough — no render-graph publish, and the audio thread is never
    // blocked on the chain edit.
    if let Some(track) = ctx.tracks().get(&track_id) {
        ctx.shared
            .retired
            .retire(track.retain_plugins(|&id| id != instance_id));
    }
    // Unpublish the instance. The slot is retired, not dropped: the
    // engine loop's sweep destroys it once no block pins it (B-4).
    remove_plugin_slots(ctx.shared, &[instance_id]);
    let _ = ctx.event_tx.send(AudioEvent::PluginRemoved {
        track_id,
        instance_id,
    });
}

pub(crate) fn handle_move_plugin(
    ctx: &HandlerCtx,
    track_id: TrackId,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    // Same shape as `handle_remove_plugin`: `move_plugin` builds the
    // reordered Vec here on the engine thread and publishes it with one
    // `ArcSwap::store`, so reading the published track map is enough. The
    // audio thread is never blocked on the edit and never allocates, and
    // no lock is held across a `process()` call — the plugin instances
    // themselves are untouched, only the order they are visited in.
    let moved = ctx
        .tracks()
        .get(&track_id)
        .and_then(|track| {
            track.move_plugin_into(instance_id, to_index, |old| ctx.shared.retired.retire(old))
        });
    match moved {
        // Report the *clamped* index so the app mirrors what the engine
        // actually did rather than what was requested.
        Some(to_index) => {
            let _ = ctx.event_tx.send(AudioEvent::PluginMoved {
                track_id,
                instance_id,
                to_index,
            });
        }
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::not_found(format!(
                "Cannot reorder plugin {} on track {}: no such track, or that \
                 plugin is not on its chain",
                instance_id, track_id
            ))));
        }
    }
}

/// Answer an `AudioCommand::ResolvePluginParamText` query with an
/// `AudioEvent::PluginParamTextResolved`: which value does the plugin say
/// `text` names, and does its own display of that value say `text` back?
/// Held under the instance lock, like `param_text`, because both calls are
/// main-thread calls that must not race `process()`; when the audio thread
/// holds the lock the query is re-enqueued rather than blocking it,
/// exactly as a parameter set is.
pub(crate) fn handle_resolve_param_text(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    param_id: u32,
    text: String,
    token: u64,
) {
    let answer = |value: Option<f64>| {
        let _ = ctx
            .event_tx
            .send(AudioEvent::PluginParamTextResolved { token, value });
    };
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        if let Some(inst) = mutex.try_lock() {
            let value = inst.0.param_from_text(param_id, &text).filter(|&v| {
                inst.0
                    .param_text(param_id, v)
                    .is_some_and(|shown| crate::label_round_trips(&text, &shown))
            });
            answer(value);
        } else {
            let _ = ctx.cmd_tx_retry.send(AudioCommand::ResolvePluginParamText {
                instance_id,
                param_id,
                text,
                token,
            });
        }
    } else {
        answer(None);
    }
}

pub(crate) fn handle_set_plugin_param(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    param_id: u32,
    value: f64,
) {
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        if let Some(mut inst) = mutex.try_lock() {
            inst.0.set_param(param_id, value);
            // `set_param` only queues; the queue is drained inside
            // `process()`. The mixer skips the whole arrangement render
            // while the transport is stopped (and skips silenced
            // non-instrument tracks while it rolls), so without this the
            // change could sit queued indefinitely — stale DSP, and a
            // `save_state()` taken in that window serialising the OLD
            // value. `clap_plugin_params.flush` is CLAP's entry point for
            // exactly that case. It drains the queue, so a change is never
            // both flushed and replayed on the next `process()`; if the
            // plugin implements no `flush`, the queue is left intact and
            // the next `process()` still applies it.
            //
            // Threading: `flush` must not run concurrently with
            // `process()`. We hold this instance's mutex — the same lock
            // every `process()` call site takes — across the call, which
            // is what guarantees that. See `clap_host::params`.
            inst.0.flush_pending_params();
            // Whatever the flush made the plugin report reaches the app
            // through the host-request poll.
            inst.0.publish_out_events();

            // Tell the app what the plugin CALLS this value (ba todo
            // #1290). The app's parameter cache is filled once, at
            // instantiation, so it holds the load-time formatting
            // forever; only the plugin can produce the new one. Sent
            // while we still hold the instance lock, for the same reason
            // `flush` is: `value_to_text` is a main-thread call that must
            // not race `process()`.
            if let Some(text) = inst.0.param_text(param_id, value) {
                let _ = ctx.event_tx.send(AudioEvent::PluginParamText {
                    instance_id,
                    param_id,
                    value,
                    text,
                });
            }
        } else {
            // Audio thread is mid-process(): re-enqueue so the param
            // change lands on the next iteration rather than blocking
            // here. Blocking causes the audio thread's own try_lock to
            // start failing too, which silences the plugin for a block.
            let _ = ctx.cmd_tx_retry.send(AudioCommand::SetPluginParam {
                instance_id,
                param_id,
                value,
            });
        }
    }
}

/// Apply a bypass request to one [`BypassFade`], choosing between the
/// crossfade and an immediate landing.
///
/// A crossfade only earns its keep when there is audio to protect. While
/// nothing is rendering — transport stopped and no input monitoring — the
/// change lands outright, which is what makes **project load correct**:
/// the app restores saved bypass state through the very same commands a
/// user toggle uses, and a restore must not spend the first few
/// milliseconds of playback in the state the project was *not* saved in.
///
/// Shared by the track, bus, master and per-slot handlers so the four
/// cannot drift.
pub fn apply_bypass_request(
    shared: &super::SharedState,
    fade: &crate::bypass::BypassFade,
    bypassed: bool,
) {
    use std::sync::atomic::Ordering;
    let rendering =
        shared.playing.load(Ordering::Relaxed) || shared.monitoring.load(Ordering::Relaxed);
    if rendering {
        fade.set_bypassed(bypassed);
    } else {
        fade.set_bypassed_settled(bypassed);
    }
}

/// Bypass (or re-engage) one chain slot — ba doc #275 finding X3.
///
/// Nothing about the audio switches here: the flag the render path fades
/// towards is set, and the mixer crossfades over the next few
/// milliseconds. The instance is *not* locked, so this can never contend
/// with the audio callback and never needs the re-enqueue dance the
/// parameter / state handlers do.
///
/// A slot whose plugin declares its own bypass parameter keeps running
/// while bypassed, so its latency stays in the chain; a slot the host
/// skips loses its latency. Either way the caller (the engine loop, via
/// [`affects_latency`]) republishes the compensation table right after
/// this returns, so PDC is correct across the toggle.
pub(crate) fn handle_set_plugin_bypass(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    bypassed: bool,
) {
    let own_bypass_param = {
        let plugins_guard = ctx.plugins();
        let Some(slot) = plugins_guard.get(&instance_id) else {
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::not_found(format!(
                "Cannot bypass plugin {}: no such plugin instance",
                instance_id
            ))));
            return;
        };
        apply_bypass_request(ctx.shared, &slot.bypass, bypassed);
        slot.bypass_param.is_some()
    };
    let _ = ctx.event_tx.send(AudioEvent::PluginBypassChanged {
        instance_id,
        bypassed,
        own_bypass_param,
    });
}

/// The pair of events that report a failed editor open (ba todo #1347).
///
/// Split out as a pure function so the wording and the structured
/// payload cannot drift, and so `tests/clap_host/plugin_editor_state.rs` can pin
/// the failure path — the one that used to lie, reporting only
/// `AudioEvent::Error("Failed to open plugin editor")` with no instance
/// id and no reason.
///
/// Two events, deliberately:
/// * [`AudioEvent::PluginEditorState`] is the correlatable truth the app
///   mirrors onto its slot (`open: false`, so an optimistic "editor is
///   open" is corrected);
/// * [`AudioEvent::Error`] keeps the existing user-facing banner working
///   — but it now names the instance and the reason instead of a bare
///   string. Once the app renders its own message from the structured
///   event (ba todo #1306) this second event can go.
pub fn plugin_editor_failure_events(
    instance_id: PluginInstanceId,
    failure: PluginEditorFailure,
) -> [AudioEvent; 2] {
    [
        AudioEvent::PluginEditorState {
            instance_id,
            open: false,
            failure: Some(failure),
        },
        AudioEvent::Error(EngineError::new(
            failure.engine_error_kind(),
            format!(
                "Could not open the editor for plugin instance {}: {}.",
                instance_id,
                failure.message()
            ),
        )),
    ]
}

/// Deadlock rule for editor open/close (macos-editor-plan.md §1): on
/// macOS the editor runtime dispatches window work to the AppKit main
/// thread and blocks until it is serviced, so the engine control thread
/// — CLAP's "main thread" for hosted plugins, i.e. this handler — must
/// be a dedicated thread distinct from the process main thread, and the
/// process main thread must never block on the engine control thread
/// (today the app only sends non-blocking `AudioCommand`s, so it
/// doesn't). Asserted in debug builds; a violation would present as the
/// teardown wedge `editor_open` guards against, not an error.
fn debug_assert_editor_deadlock_rule() {
    #[cfg(target_os = "macos")]
    // SAFETY: pthread_main_np takes no arguments and only inspects the
    // calling thread; it returns non-zero iff this is the main thread.
    debug_assert!(
        unsafe { libc::pthread_main_np() } == 0,
        "plugin editor open/close must run on the engine control thread, \
         not the process main thread (see the deadlock rule above)"
    );
}

pub(crate) fn handle_open_plugin_editor(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    debug_assert_editor_deadlock_rule();
    let mut outcome = Err(PluginEditorFailure::UnknownInstance);
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        // open_gui is a main-thread operation; the audio thread holds
        // a different lock. Block briefly if the audio thread is
        // mid-process and retry.
        if let Some(mut inst) = mutex.try_lock() {
            outcome = inst.0.open_gui();
        } else {
            // Deferred, not decided: the retry emits the event.
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::OpenPluginEditor { instance_id });
            return;
        }
    }
    match outcome {
        Ok(()) => {
            let _ = ctx.event_tx.send(AudioEvent::PluginEditorState {
                instance_id,
                open: true,
                failure: None,
            });
        }
        Err(failure) => {
            for event in plugin_editor_failure_events(instance_id, failure) {
                let _ = ctx.event_tx.send(event);
            }
        }
    }
}

pub(crate) fn handle_close_plugin_editor(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    debug_assert_editor_deadlock_rule();
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        if let Some(mut inst) = mutex.try_lock() {
            // Whether this actually closed an open editor or was a
            // no-op, the reported state below is the same: closed.
            let _ = inst.0.close_gui();
        } else {
            // Deferred: the retry reports the close.
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::ClosePluginEditor { instance_id });
            return;
        }
    }
    // Reported unconditionally, including for an instance the engine
    // does not have and for a close that was already a no-op: "closed"
    // is the truth in every one of those cases, and an app mirror that
    // set `editor_open` optimistically needs it cleared.
    let _ = ctx.event_tx.send(AudioEvent::PluginEditorState {
        instance_id,
        open: false,
        failure: None,
    });
}

/// Sync the plugin's values under the instance lock (a `params.flush`,
/// cheap), then hand back what `handle` takes from it, for serialising
/// outside the lock (code review HOST-03): a large state — user
/// wavetables, a drum kit — took long enough to save that the audio
/// thread skipped the plugin's blocks while the engine held the lock.
/// `None` when the lock is busy.
fn sync_then_unlock<H>(
    mutex: &parking_lot::Mutex<crate::clap_host::SyncClapInstance>,
    handle: impl FnOnce(&mut crate::clap_host::ClapInstance) -> H,
) -> Option<H> {
    let mut inst = mutex.try_lock()?;
    // An editor edit made with the transport stopped is in the plugin
    // but not yet in what it saves (HOST-01).
    inst.0.sync_plugin_values();
    inst.0.publish_out_events();
    Some(handle(&mut inst.0))
}

pub(crate) fn handle_save_plugin_state(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        if let Some(handle) = sync_then_unlock(mutex, |inst| inst.state_save_handle()) {
            // SAFETY: engine thread; the instance stays in `ctx.plugins()`
            // for the whole command (only the engine thread removes one).
            if let Some(data) = handle.and_then(|h| unsafe { h.save() }) {
                let _ = ctx
                    .event_tx
                    .send(AudioEvent::PluginStateSaved { instance_id, data });
            }
        } else {
            // Audio thread holds the lock — retry next tick
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::SavePluginState { instance_id });
        }
    }
}

pub(crate) fn handle_load_plugin_state(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    data: Vec<u8>,
) {
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        if let Some(mut inst) = mutex.try_lock() {
            if let Some(event) = reload_plugin_state(&mut inst.0, instance_id, &data) {
                let _ = ctx.event_tx.send(event);
            }
        } else {
            // Audio thread holds the lock — retry next tick
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::LoadPluginState { instance_id, data });
        }
    }
}

pub(crate) fn handle_save_plugin_preset_state(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        let handle = sync_then_unlock(mutex, |inst| {
            (inst.preset_save_handle(), inst.is_first_party())
        });
        if let Some((handle, first_party)) = handle {
            // SAFETY: as in `handle_save_plugin_state`.
            if let Some((data, preset_form)) = unsafe { handle.save() } {
                let _ = ctx.event_tx.send(AudioEvent::PluginPresetStateSaved {
                    instance_id,
                    data,
                    preset_form,
                    first_party,
                });
            }
        } else {
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::SavePluginPresetState { instance_id });
        }
    }
}

/// A capture token whose "before" state was already sent (a load retried
/// after the lock was busy): only the "after" state is still owed.
const CAPTURE_AFTER_ONLY: u64 = 1 << 63;

/// Save and send the full state for `token`, without holding the
/// instance's lock (a user-wavetable state is megabytes of base64; holding
/// the lock through it made the audio thread skip the plugin's blocks).
fn capture_unlocked(
    ctx: &HandlerCtx,
    mutex: &parking_lot::Mutex<crate::clap_host::SyncClapInstance>,
    instance_id: PluginInstanceId,
    token: u64,
    after: bool,
) -> bool {
    // Synced under the lock (a `params.flush`, cheap), saved outside it.
    let Some(handle) = sync_then_unlock(mutex, |inst| inst.state_save_handle()) else {
        return false;
    };
    // SAFETY: engine thread; the instance stays in `ctx.plugins()` for the
    // whole command (only the engine thread removes one).
    if let Some(data) = handle.and_then(|h| unsafe { h.save() }) {
        let _ = ctx.event_tx.send(AudioEvent::PluginStateCaptured {
            instance_id,
            token,
            data,
            after,
        });
    }
    true
}

/// The "before" half of a capture; returns the token still owing its
/// "after" half, or `Err(())` when the lock was busy (retry the command).
fn capture_before(
    ctx: &HandlerCtx,
    mutex: &parking_lot::Mutex<crate::clap_host::SyncClapInstance>,
    instance_id: PluginInstanceId,
    capture: Option<u64>,
) -> Result<Option<u64>, ()> {
    match capture {
        None => Ok(None),
        Some(t) if t & CAPTURE_AFTER_ONLY != 0 => Ok(Some(t & !CAPTURE_AFTER_ONLY)),
        Some(t) => {
            if capture_unlocked(ctx, mutex, instance_id, t, false) {
                Ok(Some(t))
            } else {
                Err(())
            }
        }
    }
}

/// The "after" half, once the load is done (best effort: a busy lock
/// retries it a few times, then gives up — redo then falls back to the
/// cached state).
fn capture_after(
    ctx: &HandlerCtx,
    mutex: &parking_lot::Mutex<crate::clap_host::SyncClapInstance>,
    instance_id: PluginInstanceId,
    token: Option<u64>,
) {
    let Some(token) = token else {
        return;
    };
    for _ in 0..8 {
        if capture_unlocked(ctx, mutex, instance_id, token, true) {
            return;
        }
        std::thread::yield_now();
    }
}

/// The mirror follows what the plugin actually took (an opaque preset's
/// values are known only to the plugin): now, and once more after the
/// next block, for a plugin that applies a load there.
fn refresh_params(
    ctx: &HandlerCtx,
    inst: &mut crate::clap_host::ClapInstance,
    instance_id: PluginInstanceId,
) {
    let _ = ctx.event_tx.send(AudioEvent::PluginParamsRefreshed {
        instance_id,
        params: inst.query_params(),
    });
    inst.request_params_refresh();
}

pub(crate) fn handle_load_plugin_preset_state(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    data: Vec<u8>,
    capture: Option<u64>,
) {
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        let Ok(after) = capture_before(ctx, mutex, instance_id, capture) else {
            let _ = ctx.cmd_tx_retry.send(AudioCommand::LoadPluginPresetState {
                instance_id,
                data,
                capture,
            });
            return;
        };
        if let Some(mut inst) = mutex.try_lock() {
            // A plugin with state-context lays the preset over its state
            // with no reactivation; any other takes it as a full state,
            // through the same reload (and failure messages) a project
            // state load uses.
            let loaded = if inst.0.has_preset_state() {
                let ok = inst.0.load_preset_state(&data);
                if !ok {
                    let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::plugin(format!(
                        "Plugin instance {instance_id} rejected the preset it was given; \
                         it keeps its previous settings."
                    ))));
                }
                ok
            } else {
                match reload_plugin_state(&mut inst.0, instance_id, &data) {
                    Some(event) => {
                        let _ = ctx.event_tx.send(event);
                        false
                    }
                    None => true,
                }
            };
            if loaded {
                refresh_params(ctx, &mut inst.0, instance_id);
            }
            drop(inst);
            capture_after(ctx, mutex, instance_id, after);
        } else {
            let _ = ctx.cmd_tx_retry.send(AudioCommand::LoadPluginPresetState {
                instance_id,
                data,
                capture: after.map(|t| t | CAPTURE_AFTER_ONLY),
            });
        }
    }
}

pub(crate) fn handle_set_plugin_preset_ignored_params(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    clap_ids: Vec<u32>,
) {
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        if let Some(mut inst) = mutex.try_lock() {
            inst.0.set_preset_ignored_params(&clap_ids);
        } else {
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::SetPluginPresetIgnoredParams { instance_id, clap_ids });
        }
    }
}

pub(crate) fn handle_load_plugin_preset_from_location(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    location: crate::types::PluginPresetLocation,
    load_key: Option<String>,
    capture: Option<u64>,
) {
    if let Some(mutex) = ctx.plugins().get(&instance_id) {
        let Ok(after) = capture_before(ctx, mutex, instance_id, capture) else {
            let _ = ctx.cmd_tx_retry.send(AudioCommand::LoadPluginPresetFromLocation {
                instance_id,
                location,
                load_key,
                capture,
            });
            return;
        };
        if let Some(mut inst) = mutex.try_lock() {
            if inst.0.load_preset_from_location(&location, load_key.as_deref()) {
                refresh_params(ctx, &mut inst.0, instance_id);
            } else {
                // One error, not two: the plugin's own `on_error` message
                // (queued by the call) is folded into it.
                let plugin_says = inst.0.take_preset_error();
                let detail = plugin_says.map(|m| format!(": {m}")).unwrap_or_default();
                let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::plugin(format!(
                    "Plugin instance {instance_id} could not load the preset it was asked \
                     for{detail}"
                ))));
            }
            drop(inst);
            capture_after(ctx, mutex, instance_id, after);
        } else {
            let _ = ctx.cmd_tx_retry.send(AudioCommand::LoadPluginPresetFromLocation {
                instance_id,
                location,
                load_key,
                capture: after.map(|t| t | CAPTURE_AFTER_ONLY),
            });
        }
    }
}

/// Reload `inst` from a state blob and describe a failure as the
/// user-visible error the engine should emit, or `None` on success
/// (code review ENG-02: a rejected preset used to leave the plugin
/// silently deactivated). `pub` via `test_support` — see
/// `tests/clap_host/clap_latency_tracking.rs`.
pub fn reload_plugin_state(
    inst: &mut crate::clap_host::ClapInstance,
    instance_id: PluginInstanceId,
    data: &[u8],
) -> Option<AudioEvent> {
    if inst.reload_with_state(data) {
        return None;
    }
    Some(AudioEvent::Error(EngineError::plugin(if inst.is_active() {
        format!(
            "Plugin instance {} rejected the state it was given (corrupt preset, or saved by \
             another plugin version); it keeps its previous settings.",
            instance_id
        )
    } else {
        format!(
            "Plugin instance {} failed to reactivate after a state load; it is deactivated \
             and will stay silent.",
            instance_id
        )
    })))
}

pub(crate) fn handle_save_all_plugin_states(ctx: &HandlerCtx) {
    let mut states = Vec::new();
    let plugins_guard = ctx.plugins();
    let mut retry = false;
    for (&instance_id, mutex) in plugins_guard.iter() {
        // What the plugin plays, not what it last reported (HOST-01,
        // PUX-01): an editor edit the plugin never announced is in its
        // params, and reaches neither its saved state nor the app's mirror
        // on its own while the transport is stopped. The flush brings its
        // saved state in step; the values that moved since the host last
        // read them go to the app ahead of the states, on the same
        // channel, so the project file the app writes from its mirror when
        // the states arrive carries them — not stale values it would
        // re-send over the blob on reopen. Both under the lock; the save
        // itself outside it (HOST-03).
        let synced = sync_then_unlock(mutex, |inst| {
            (inst.refresh_param_values(false), inst.state_save_handle())
        });
        let Some((values, handle)) = synced else {
            retry = true;
            break;
        };
        if !values.is_empty() {
            let _ = ctx
                .event_tx
                .send(AudioEvent::PluginParamValuesChanged { instance_id, values });
        }
        // SAFETY: as in `handle_save_plugin_state`.
        if let Some(data) = handle.and_then(|h| unsafe { h.save() }) {
            states.push((instance_id, data));
        }
    }
    drop(plugins_guard);
    if retry {
        let _ = ctx.cmd_tx_retry.send(AudioCommand::SaveAllPluginStates);
    } else {
        let _ = ctx
            .event_tx
            .send(AudioEvent::AllPluginStatesSaved { states });
    }
}

/// Returns the index of the bundle to instantiate `clap_plugin_id` from,
/// loading the file from disk if needed. `Err` carries the loader's
/// reason; the caller turns it into a [`AudioEvent::PluginLoadFailed`]
/// naming the slot that stays empty.
///
/// Keyed by the file first, the id second (code review HOST-10): the same
/// CLAP id can live in two bundles — a dev `target/bundled` build beside
/// an installed copy — and the slot must get the one it names. So:
///
/// 1. a loaded bundle at `path` (canonicalised, as the scanner stores it)
///    that has the id — or any loaded bundle at `path`, for an empty id,
///    which used to re-`dlopen` the file on every add;
/// 2. else the file at `path`, loaded now, if it exists and has the id;
/// 3. else any loaded bundle with the id — a project saved before its
///    plugin moved still finds it;
/// 4. else the file's own load error (or the bundle it loaded, whose
///    missing id the caller then reports).
pub fn ensure_bundle(
    bundles: &mut Vec<ClapBundle>,
    path: &Path,
    clap_plugin_id: &str,
) -> Result<usize, PluginBundleError> {
    let has_id = |b: &ClapBundle| {
        clap_plugin_id.is_empty() || b.descriptors().iter().any(|d| d.id == clap_plugin_id)
    };
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let canonical_str = canonical.to_string_lossy();
    let at_path = bundles.iter().position(|b| b.path() == canonical_str);
    if let Some(idx) = at_path.filter(|&i| has_id(&bundles[i])) {
        return Ok(idx);
    }
    let loaded = match at_path {
        Some(idx) => Ok(idx),
        None => ClapBundle::load(&canonical).map(|bundle| {
            bundles.push(bundle);
            bundles.len() - 1
        }),
    };
    if let Ok(idx) = loaded {
        if has_id(&bundles[idx]) {
            return Ok(idx);
        }
    }
    if let Some(idx) = bundles.iter().position(|b| has_id(b) && !clap_plugin_id.is_empty()) {
        return Ok(idx);
    }
    Ok(loaded?)
}

/// Returns the canonical plugin id to instantiate from `bundle`. If the
/// caller passed an empty id, pick the first descriptor; otherwise hand
/// back the id as-is.
pub(crate) fn resolve_plugin_id(
    bundle: &ClapBundle,
    clap_plugin_id: String,
) -> Result<String, PluginBundleError> {
    if !clap_plugin_id.is_empty() {
        return Ok(clap_plugin_id);
    }
    match bundle.descriptors().first() {
        Some(d) => Ok(d.id.clone()),
        None => Err(PluginBundleError::NoPluginsFound),
    }
}

/// Report an add that produced no instance, on any of the three chains.
///
/// One event, not two: this REPLACES the bare
/// [`AudioEvent::Error`](AudioEvent::Error) the three add handlers used
/// to send. That string named no instance, so the app could only show it
/// as a toast — and a project missing five plugins raised five toasts of
/// which the user saw the last, while the five dead slots looked exactly
/// like working ones. `PluginLoadFailed` carries the same words plus the
/// identity needed to mark the slot, and the app decides how to present
/// it (badge + load warning when it maps to a slot, plain error when it
/// does not).
pub(crate) fn report_plugin_load_failure(
    ctx: &HandlerCtx,
    id_hint: Option<PluginInstanceId>,
    clap_plugin_id: &str,
    clap_file_path: &str,
    reason: String,
) {
    let _ = ctx.event_tx.send(plugin_load_failed_event(
        id_hint,
        clap_plugin_id,
        clap_file_path,
        reason,
    ));
}

/// Build the failure event, separately from sending it, so the one thing
/// that makes it useful can be pinned by a test without an engine
/// thread: **the event carries the instance id the command asked for**.
///
/// That is the whole difference from the `AudioEvent::Error` string this
/// replaced. Drop `id_hint` on the floor here and the app is back to
/// knowing that *something* failed and having no idea which slot to
/// mark — which is the bug (ba doc #275 P5, todo #1309).
pub fn plugin_load_failed_event(
    id_hint: Option<PluginInstanceId>,
    clap_plugin_id: &str,
    clap_file_path: &str,
    reason: String,
) -> AudioEvent {
    AudioEvent::PluginLoadFailed {
        instance_id: id_hint,
        clap_plugin_id: clap_plugin_id.to_string(),
        clap_file_path: clap_file_path.to_string(),
        reason,
    }
}
