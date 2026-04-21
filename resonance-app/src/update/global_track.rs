use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{GlobalTrackMessage, Message};
use crate::state;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: GlobalTrackMessage) -> Task<Message> {
    match m {
        GlobalTrackMessage::AddTempoEvent { bar, bpm } => {
            r.tempo_events.push(state::TempoEvent { bar, bpm });
            r.tempo_events.sort_by_key(|e| e.bar);
            r.rebuild_and_send_tempo();
            r.sync_tempo_display();
        }
        GlobalTrackMessage::StartTempoDrag(index) => {
            r.interaction.selected_global_event = Some(state::SelectedGlobalEvent {
                kind: state::GlobalTrackKind::Tempo,
                index,
            });
        }
        GlobalTrackMessage::EndTempoDrag => {
            r.tempo_events.sort_by_key(|e| e.bar);
            r.rebuild_and_send_tempo();
        }
        GlobalTrackMessage::UpdateTempoEvent { index, bar, bpm } => {
            let bpm = bpm.clamp(20.0, 300.0);
            if let Some(event) = r.tempo_events.get_mut(index) {
                event.bar = if index == 0 { 0 } else { bar };
                event.bpm = bpm;
                r.rebuild_tempo_map();
                r.sync_tempo_display();
                r.engine.send(AudioCommand::SetBpm {
                    bpm: r.transport.bpm,
                });
            }
        }
        GlobalTrackMessage::AddSignatureEvent {
            bar,
            numerator,
            denominator,
        } => {
            if let Some(existing) = r.signature_events.iter_mut().find(|e| e.bar == bar) {
                existing.numerator = numerator;
                existing.denominator = denominator;
            } else {
                r.signature_events.push(state::SignatureEvent {
                    bar,
                    numerator,
                    denominator,
                });
                r.signature_events.sort_by_key(|e| e.bar);
            }
            r.rebuild_and_send_tempo();
            let event_sample = r.tempo_map.bar_to_sample(bar);
            if r.transport.playhead >= event_sample {
                r.transport.time_sig_num = numerator;
                r.transport.time_sig_den = denominator;
                r.engine.send(AudioCommand::SetTimeSignature {
                    numerator,
                    denominator,
                });
            }
            if let Some(idx) = r.signature_events.iter().position(|e| e.bar == bar) {
                r.interaction.selected_global_event = Some(state::SelectedGlobalEvent {
                    kind: state::GlobalTrackKind::Signature,
                    index: idx,
                });
            }
        }
        GlobalTrackMessage::UpdateSignatureEvent {
            index,
            numerator,
            denominator,
        } => {
            if let Some(event) = r.signature_events.get_mut(index) {
                event.numerator = numerator;
                event.denominator = denominator;
            }
            r.rebuild_and_send_tempo();
            if let Some(event) = r.signature_events.get(index) {
                let event_sample = r.tempo_map.bar_to_sample(event.bar);
                if r.transport.playhead >= event_sample {
                    r.transport.time_sig_num = numerator;
                    r.transport.time_sig_den = denominator;
                    r.engine.send(AudioCommand::SetTimeSignature {
                        numerator,
                        denominator,
                    });
                }
            }
        }
        GlobalTrackMessage::SelectEvent(sel) => {
            r.interaction.selected_global_event = sel;
        }
        GlobalTrackMessage::DeleteSelectedEvent => {
            if let Some(sel) = r.interaction.selected_global_event.take() {
                match sel.kind {
                    state::GlobalTrackKind::Tempo => r.remove_tempo_event(sel.index),
                    state::GlobalTrackKind::Signature => r.remove_signature_event(sel.index),
                }
            }
        }
    }
    Task::none()
}
