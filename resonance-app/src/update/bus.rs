use iced::Task;
use resonance_audio::types::{AudioCommand, TrackOutput};

use crate::message::{BusMessage, Message};
use crate::util::db_to_gain;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: BusMessage) -> Task<Message> {
    match m {
        BusMessage::AddBus => {
            let _ = r.engine.send(AudioCommand::AddBus {
                id_hint: None,
                name: None,
            });
        }
        BusMessage::AddBusWithId { id, name } => {
            let _ = r.engine.send(AudioCommand::AddBus {
                id_hint: Some(id),
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
                r.view_caches.rebuild_output(&r.registry.busses);
            }
        }
        BusMessage::RemoveBus(bus_id) => {
            let _ = r.engine.send(AudioCommand::RemoveBus { bus_id });
            for track in &mut r.registry.tracks {
                if track.output == TrackOutput::Bus(bus_id) {
                    track.output = TrackOutput::Master;
                }
            }
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
            let _ = r.engine.send(AudioCommand::AddPluginToBus {
                bus_id,
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id_hint: None,
            });
        }
        BusMessage::RemovePluginFromBus(bus_id, instance_id) => {
            let _ = r.engine.send(AudioCommand::RemovePluginFromBus {
                bus_id,
                instance_id,
            });
        }
    }
    Task::none()
}
