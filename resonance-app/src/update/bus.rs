use iced::Task;
use resonance_audio::types::{AudioCommand, TrackOutput};

use crate::message::{BusMessage, Message};
use crate::util::db_to_gain;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: BusMessage) -> Task<Message> {
    match m {
        BusMessage::AddBus => {
            r.engine.send(AudioCommand::AddBus {
                id_hint: None,
                name: None,
            });
        }
        BusMessage::RemoveBus(bus_id) => {
            r.engine.send(AudioCommand::RemoveBus { bus_id });
            for track in &mut r.registry.tracks {
                if track.output == TrackOutput::Bus(bus_id) {
                    track.output = TrackOutput::Master;
                }
            }
        }
        BusMessage::SetBusVolume(bus_id, vol_db) => {
            r.engine.send(AudioCommand::SetBusVolume {
                bus_id,
                volume: db_to_gain(vol_db),
            });
            r.with_bus_mut(bus_id, |b| b.volume = vol_db);
        }
        BusMessage::SetBusPan(bus_id, pan) => {
            r.engine.send(AudioCommand::SetBusPan { bus_id, pan });
            r.with_bus_mut(bus_id, |b| b.pan = pan);
        }
        BusMessage::ToggleBusMute(bus_id) => {
            let new_muted = r.with_bus_mut(bus_id, |b| {
                b.muted = !b.muted;
                b.muted
            });
            if let Some(muted) = new_muted {
                r.engine
                    .send(AudioCommand::SetBusMute { bus_id, muted });
            }
        }
        BusMessage::ToggleBusFxBypass(bus_id) => {
            let new_bypass = r.with_bus_mut(bus_id, |b| {
                b.fx_bypassed = !b.fx_bypassed;
                b.fx_bypassed
            });
            if let Some(bypassed) = new_bypass {
                r.engine
                    .send(AudioCommand::SetBusFxBypass { bus_id, bypassed });
            }
        }
        BusMessage::AddPluginToBus(bus_id, plugin) => {
            r.engine.send(AudioCommand::AddPluginToBus {
                bus_id,
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id_hint: None,
            });
        }
        BusMessage::RemovePluginFromBus(bus_id, instance_id) => {
            r.engine.send(AudioCommand::RemovePluginFromBus {
                bus_id,
                instance_id,
            });
        }
    }
    Task::none()
}
