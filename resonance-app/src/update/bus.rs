use iced::Task;
use resonance_audio::types::{AudioCommand, BusId, PluginInstanceId, ScannedPlugin};

use crate::message::Message;
use crate::util::db_to_gain;
use crate::Resonance;

/// `Message::Bus` variants, handled by [`handle`] in this module.
/// Declared here beside its handler and re-exported from `crate::message`
/// (ARCH-01 A1-3).
#[derive(Debug, Clone)]
pub enum BusMessage {
    /// Add a bus with an app-generated default name. Since ARCH-04 D-3
    /// the id is app-allocated here too (`Resonance::allocate_bus_id`),
    /// same as [`AddBusWithId`](Self::AddBusWithId) — what distinguishes
    /// the two is only that this one waits for the engine's `BusAdded`
    /// echo to mirror the bus, rather than mirroring it eagerly.
    AddBus,
    /// Add a bus whose id and name the *app* chose up front, so the
    /// caller can use the id without waiting for the engine's `BusAdded`
    /// echo. Same pattern as
    /// [`MixerMessage::CreateReturnFromSend`](crate::message::MixerMessage::CreateReturnFromSend)
    /// and the control API's `track.add`.
    AddBusWithId { id: BusId, name: String },
    RemoveBus(BusId),
    SetBusVolume(BusId, f32),
    SetBusPan(BusId, f32),
    ToggleBusMute(BusId),
    ToggleBusFxBypass(BusId),
    AddPluginToBus(BusId, ScannedPlugin),
    /// Add a plugin to a bus whose instance id the *app* chose up front,
    /// mirroring a placeholder slot into `BusState.plugins` immediately
    /// so the caller can address it without waiting for the engine's
    /// `BusPluginAdded` echo (ba doc #273, todo #1237). The bus twin of
    /// [`PluginMessage::AddPluginToTrackWithId`](crate::message::PluginMessage::AddPluginToTrackWithId);
    /// `engine_events::plugins::bus_added` is idempotent, so the echo
    /// fills the placeholder's params in rather than pushing a
    /// duplicate. The GUI never sends this.
    AddPluginToBusWithId {
        bus_id: BusId,
        instance_id: PluginInstanceId,
        plugin: ScannedPlugin,
    },
    RemovePluginFromBus(BusId, PluginInstanceId),
    /// Reorder a bus's insert chain: move `instance_id` to `to_index`,
    /// clamped to the last slot. Sends `AudioCommand::MovePluginInBus`
    /// AND mirrors the new order into `BusState.plugins`, so a control
    /// client reads its own write back in the same cycle; the engine's
    /// `BusPluginMoved` echo replays the same move and is then a no-op.
    MovePluginInBus {
        bus_id: BusId,
        instance_id: PluginInstanceId,
        to_index: usize,
    },
}

impl BusMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            Self::SetBusVolume(id, _) => UndoAction::RecordCoalesced(CoalesceKey::BusVolume(*id)),
            Self::SetBusPan(id, _) => UndoAction::RecordCoalesced(CoalesceKey::BusPan(*id)),
            // Every other variant is a discrete, persisted edit.
            Self::AddBus
            | Self::AddBusWithId { .. }
            | Self::RemoveBus(..)
            | Self::ToggleBusMute(..)
            | Self::ToggleBusFxBypass(..)
            | Self::AddPluginToBus(..)
            | Self::AddPluginToBusWithId { .. }
            | Self::RemovePluginFromBus(..)
            | Self::MovePluginInBus { .. } => UndoAction::Record,
        }
    }
}

pub fn handle(r: &mut Resonance, m: BusMessage) -> Task<Message> {
    match m {
        BusMessage::AddBus => {
            // App-allocated (ARCH-04 D-3); still no eager mirror, same as
            // the plain plugin/send GUI adds — the mixer waits for
            // `BusAdded`.
            let id = r.registry.allocate_bus_id();
            let _ = r.engine.send(AudioCommand::AddBus { id, name: None });
        }
        BusMessage::AddBusWithId { id, name } => {
            let _ = r.engine.send(AudioCommand::AddBus {
                id,
                name: Some(name.clone()),
            });
            // Mirror the bus NOW rather than on the engine's `BusAdded`
            // echo (ba doc #273, todo #1238 item 1). The caller was
            // handed this id in a reply, and `track.set_output` /
            // `track.add_send` validate against `registry.busses`, so
            // routing into it on the very next call must be guaranteed,
            // not merely likely. `engine_events::tracks::bus_added`
            // returns early when the id is already present, so the echo
            // is a no-op — the same idempotency the plugin add relies on
            // (todo #1234).
            if !r.registry.busses.iter().any(|b| b.id == id) {
                let order = r.registry.next_bus_order;
                r.registry.next_bus_order += 1;
                r.registry
                    .busses
                    .push(crate::state::BusState::new(id, order, name));
                r.registry.resort_busses();
                r.ui.view_caches.rebuild_output(&r.registry.busses);
            }
        }
        BusMessage::RemoveBus(bus_id) => {
            // Mirror the removal now, not on the `BusRemoved` echo, so an
            // undo pressed before the echo lands sees it (STATE-10 shape,
            // ARCH-01 FU-A13c). The echo is owed, so a late one cannot
            // drop a bus an undo re-added under this id (A-13h).
            // `engine_events::tracks::bus_removed` also falls the bus's
            // tracks back to master, so nothing else is needed here.
            let _ = r.engine.send(AudioCommand::RemoveBus { bus_id });
            r.io.restore_echoes.expect_bus_removed(bus_id);
            crate::engine_events::tracks::bus_removed(r, bus_id);
        }
        BusMessage::SetBusVolume(bus_id, vol_db) => {
            let _ = r.engine.send(AudioCommand::SetBusVolume {
                bus_id,
                volume: db_to_gain(vol_db),
            });
            r.with_bus_mut(bus_id, |b| b.volume = vol_db);
        }
        BusMessage::SetBusPan(bus_id, pan) => {
            let _ = r.engine.send(AudioCommand::SetBusPan { bus_id, pan });
            r.with_bus_mut(bus_id, |b| b.pan = pan);
        }
        BusMessage::ToggleBusMute(bus_id) => {
            let new_muted = r.with_bus_mut(bus_id, |b| {
                b.muted = !b.muted;
                b.muted
            });
            if let Some(muted) = new_muted {
                let _ = r.engine.send(AudioCommand::SetBusMute { bus_id, muted });
            }
        }
        BusMessage::ToggleBusFxBypass(bus_id) => {
            let new_bypass = r.with_bus_mut(bus_id, |b| {
                b.fx_bypassed = !b.fx_bypassed;
                b.fx_bypassed
            });
            if let Some(bypassed) = new_bypass {
                let _ = r.engine
                    .send(AudioCommand::SetBusFxBypass { bus_id, bypassed });
            }
        }
        BusMessage::AddPluginToBus(bus_id, plugin) => {
            // App-allocated (ARCH-04 D-1); still no eager mirror, same as
            // the track GUI add — the bus chain waits for `BusPluginAdded`.
            let id = r.allocate_plugin_id();
            let _ = r.engine.send(AudioCommand::AddPluginToBus {
                bus_id,
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id,
            });
        }
        BusMessage::AddPluginToBusWithId {
            bus_id,
            instance_id,
            plugin,
        } => {
            let _ = r.engine.send(AudioCommand::AddPluginToBus {
                bus_id,
                clap_file_path: plugin.clap_file_path.clone(),
                clap_plugin_id: plugin.clap_plugin_id.clone(),
                id: instance_id,
            });
            // Mirror the slot NOW with an empty param list, as the
            // project-load replay does; `bus_added` finds it by
            // `instance_id` on the echo and fills in params/has_gui
            // instead of pushing a second one.
            if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
                bus.plugins.push(crate::state::PluginSlotState::new(
                    instance_id,
                    plugin.name,
                    plugin.clap_plugin_id,
                    plugin.clap_file_path,
                    Vec::new(),
                    false,
                ));
                r.insert_plugin_index(instance_id, crate::state::PluginLocator::Bus(bus_id));
            }
        }
        BusMessage::MovePluginInBus {
            bus_id,
            instance_id,
            to_index,
        } => {
            let _ = r.engine.send(AudioCommand::MovePluginInBus {
                bus_id,
                instance_id,
                to_index,
            });
            crate::engine_events::plugins::mirror_bus_plugin_move(
                r,
                bus_id,
                instance_id,
                to_index,
            );
        }
        BusMessage::RemovePluginFromBus(bus_id, instance_id) => {
            // Mirror the removal now, not on the `BusPluginRemoved` echo,
            // so an undo pressed before the echo lands sees it (STATE-10
            // shape, ARCH-01 FU-A13c). The echo is owed, so a late one
            // cannot drop an instance an undo re-added under this id
            // (A-13h).
            let _ = r.engine.send(AudioCommand::RemovePluginFromBus {
                bus_id,
                instance_id,
            });
            r.io.restore_echoes.expect_plugin_removed(instance_id);
            crate::engine_events::plugins::bus_removed(r, bus_id, instance_id);
        }
    }
    Task::none()
}
