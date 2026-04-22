use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{Message, TransportMessage};
use crate::state::{LoopDragTarget, ViewMode};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: TransportMessage) -> Task<Message> {
    match m {
        TransportMessage::Play => {
            // In Compose mode with a selected section, auto-loop that section
            if r.view_mode == ViewMode::Compose {
                if let Some((placement, definition)) =
                    r.compose.selected_placement().and_then(|p| {
                        r.compose.find_definition(p.definition_id).map(|d| (p, d))
                    })
                {
                    let loop_in = r.tempo_map.bar_to_sample(placement.start_bar);
                    let loop_out = r
                        .tempo_map
                        .bar_to_sample(placement.start_bar + definition.length_bars);
                    r.transport.loop_in = loop_in;
                    r.transport.loop_out = loop_out;
                    r.transport.loop_enabled = true;
                    r.transport.loop_range_set = true;
                    r.engine.send(AudioCommand::SetLoopRange {
                        enabled: true,
                        loop_in,
                        loop_out,
                    });
                    r.engine.send(AudioCommand::SeekTo(loop_in));
                    r.transport.playhead = loop_in;
                }
            }
            r.engine.send(AudioCommand::Play);
            r.transport.playing = true;
        }
        TransportMessage::Record => {
            if r.registry.tracks.iter().any(|t| t.record_armed) {
                r.engine.send(AudioCommand::Record {
                    precount_bars: r.transport.precount_bars,
                });
                r.transport.playing = true;
            }
        }
        TransportMessage::Pause => {
            r.engine.send(AudioCommand::Pause);
            r.transport.playing = false;
        }
        TransportMessage::Stop => {
            r.engine.send(AudioCommand::Stop);
            r.transport.playing = false;
            r.transport.playhead = 0;
        }
        TransportMessage::SkipBack => {
            let skip = r.sample_rate as u64 * 5;
            let new_pos = r.transport.playhead.saturating_sub(skip);
            r.engine.send(AudioCommand::SeekTo(new_pos));
            r.transport.playhead = new_pos;
        }
        TransportMessage::SkipForward => {
            let skip = r.sample_rate as u64 * 5;
            let new_pos = r.transport.playhead + skip;
            r.engine.send(AudioCommand::SeekTo(new_pos));
            r.transport.playhead = new_pos;
        }
        TransportMessage::SeekToSample(pos) => {
            r.engine.send(AudioCommand::SeekTo(pos));
            r.transport.playhead = pos;
        }
        TransportMessage::SetBpmText(s) => {
            r.transport.bpm_input = s;
        }
        TransportMessage::CommitBpm => {
            if let Ok(parsed) = r.transport.bpm_input.trim().parse::<f32>() {
                r.transport.bpm = parsed.clamp(20.0, 300.0);
                r.engine.send(AudioCommand::SetBpm {
                    bpm: r.transport.bpm,
                });
                if let Some(first) = r.tempo_events.first_mut() {
                    if first.bar == 0 {
                        first.bpm = r.transport.bpm;
                    }
                }
                r.rebuild_and_send_tempo();
            }
            r.transport.bpm_input = format!("{:.0}", r.transport.bpm);
        }
        TransportMessage::CyclePrecountBars => {
            r.transport.precount_bars = match r.transport.precount_bars {
                0 => 1,
                1 => 2,
                2 => 4,
                _ => 0,
            };
        }
        TransportMessage::ToggleMetronome => {
            r.transport.metronome_enabled = !r.transport.metronome_enabled;
            r.engine.send(AudioCommand::SetMetronomeEnabled {
                enabled: r.transport.metronome_enabled,
            });
        }
        TransportMessage::CycleTimeSignature => {
            let (num, den) = match (r.transport.time_sig_num, r.transport.time_sig_den) {
                (4, 4) => (3, 4),
                (3, 4) => (6, 8),
                (6, 8) => (5, 4),
                (5, 4) => (7, 8),
                (7, 8) => (2, 4),
                _ => (4, 4),
            };
            r.transport.time_sig_num = num;
            r.transport.time_sig_den = den;
            r.engine.send(AudioCommand::SetTimeSignature {
                numerator: num,
                denominator: den,
            });
            if let Some(first) = r.signature_events.first_mut() {
                if first.bar == 0 {
                    first.numerator = num;
                    first.denominator = den;
                }
            }
        }
        TransportMessage::ToggleLoop => {
            r.transport.loop_enabled = !r.transport.loop_enabled;
            if r.transport.loop_enabled && !r.transport.loop_range_set {
                let spb = r.sample_rate as f64 * 60.0 / r.transport.bpm as f64;
                let two_bars = (spb * r.transport.time_sig_num as f64 * 2.0) as u64;
                r.transport.loop_in = r.transport.playhead;
                r.transport.loop_out = r.transport.playhead + two_bars;
                r.transport.loop_range_set = true;
            }
            r.engine.send(AudioCommand::SetLoopRange {
                enabled: r.transport.loop_enabled,
                loop_in: r.transport.loop_in,
                loop_out: r.transport.loop_out,
            });
        }
        TransportMessage::StartLoopDrag(target) => {
            r.transport.dragging_loop = Some(target);
        }
        TransportMessage::UpdateLoopDrag(x) => {
            if r.transport.dragging_loop.is_some() {
                let seconds = (x + r.viewport.scroll_offset) / r.viewport.zoom;
                let raw = (seconds.max(0.0) as f64 * r.sample_rate as f64) as u64;
                let sample = crate::timeline::snap_sample_to_grid_tempo(
                    raw,
                    r.transport.bpm,
                    r.transport.time_sig_num,
                    r.sample_rate,
                    r.viewport.zoom,
                    &r.tempo_map,
                );
                match r.transport.dragging_loop {
                    Some(LoopDragTarget::In) => {
                        r.transport.loop_in = sample;
                    }
                    Some(LoopDragTarget::Out) => {
                        r.transport.loop_out = sample;
                    }
                    None => {}
                }
                if r.transport.loop_enabled {
                    r.engine.send(AudioCommand::SetLoopRange {
                        enabled: true,
                        loop_in: r.transport.loop_in,
                        loop_out: r.transport.loop_out,
                    });
                }
            }
        }
        TransportMessage::EndLoopDrag => {
            r.transport.dragging_loop = None;
            if r.transport.loop_in > r.transport.loop_out {
                std::mem::swap(&mut r.transport.loop_in, &mut r.transport.loop_out);
            }
            if r.transport.loop_enabled {
                r.engine.send(AudioCommand::SetLoopRange {
                    enabled: true,
                    loop_in: r.transport.loop_in,
                    loop_out: r.transport.loop_out,
                });
            }
        }
    }
    Task::none()
}
