use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{GlobalTrackMessage, Message};
use crate::state;
use crate::Resonance;

impl Resonance {
    /// Rebuild the GUI-side tempo map from the current events and send the
    /// events to the audio engine. Call whenever `tempo_events` or
    /// `signature_events` are modified.
    pub(crate) fn rebuild_and_send_tempo(&mut self) {
        self.rebuild_tempo_map();
        self.engine.send(AudioCommand::SetTempoEvents {
            tempo: self.tempo_events.clone(),
            signature: self.signature_events.clone(),
        });
    }

    /// Rebuild only the GUI-side tempo map (no engine send). Used when
    /// only UI display needs updating, e.g. during tempo drags.
    pub(crate) fn rebuild_tempo_map(&mut self) {
        self.tempo_map.tempo_points = self.tempo_events.clone();
        self.tempo_map.signature_points = self.signature_events.clone();
        self.tempo_map.bpm = self.transport.bpm;
        self.tempo_map.numerator = self.transport.time_sig_num;
        self.tempo_map.denominator = self.transport.time_sig_den;
        self.tempo_map.rebuild_bar_table(self.sample_rate);
    }

    /// Update the transport BPM display from the current tempo map.
    pub(crate) fn sync_tempo_display(&mut self) {
        let (bpm, _, _) = self
            .tempo_map
            .tempo_at_sample(self.transport.playhead, self.sample_rate);
        self.transport.bpm = bpm;
        self.transport.bpm_input = format!("{:.1}", bpm);
    }

    /// Remove a tempo event by index (must be > 0 to protect the initial
    /// event), rebuild the tempo map, and sync the BPM display.
    pub(crate) fn remove_tempo_event(&mut self, index: usize) {
        if index > 0 && index < self.tempo_events.len() {
            self.tempo_events.remove(index);
            self.rebuild_and_send_tempo();
            self.sync_tempo_display();
        }
    }

    /// Remove a signature event by index (must be > 0 to protect the initial
    /// event), rebuild the tempo map, and sync the time-signature display.
    pub(crate) fn remove_signature_event(&mut self, index: usize) {
        if index > 0 && index < self.signature_events.len() {
            self.signature_events.remove(index);
            self.rebuild_and_send_tempo();
            let (_, num, den) = self
                .tempo_map
                .tempo_at_sample(self.transport.playhead, self.sample_rate);
            self.transport.time_sig_num = num;
            self.transport.time_sig_den = den;
            self.engine.send(AudioCommand::SetTimeSignature {
                numerator: num,
                denominator: den,
            });
        }
    }
}

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
