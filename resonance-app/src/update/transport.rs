use iced::Task;
use resonance_audio::types::AudioCommand;

use super::tempo_reanchor::{musical_anchors, reanchor_to_tempo};
use crate::message::Message;
use crate::state::{LoopDragTarget, ViewMode};
use crate::Resonance;

/// `Message::Transport` variants, handled by [`handle`] in this module.
/// Declared here beside its handler and re-exported from `crate::message`
/// (ARCH-01 A1-3).
#[derive(Debug, Clone)]
pub enum TransportMessage {
    Play,
    Record,
    Pause,
    Stop,
    /// Space: play when stopped; when playing, stop and return the
    /// playhead to where playback started (command-palette.md D1).
    TogglePlay,
    /// Seek to `loop_in`, then play.
    PlayFromLoopStart,
    /// Move the playhead to a resolved target (bar / beat nudges, loop
    /// edges, project ends, section starts, a bar). One reducer, so all the
    /// seek maths lives in `transport_nav`.
    SeekTo(super::transport_nav::SeekTarget),
    /// Move one loop edge to the playhead, snapped to the grid; a crossing
    /// pushes the other edge one bar away. Does not enable the loop.
    SetLoopPoint { edge: super::transport_nav::LoopEdge },
    SkipBack,
    SkipForward,
    /// Move the playhead to the given sample position (ruler click, etc.).
    SeekToSample(u64),
    SetBpmText(String),
    CommitBpm,
    ToggleMetronome,
    CycleTimeSignature,
    /// Set the time signature directly (control endpoint, doc #265 —
    /// the GUI cycles via [`Self::CycleTimeSignature`]). Undoable like
    /// the cycle path.
    SetTimeSignature { numerator: u8, denominator: u8 },
    ToggleLoop,
    /// Set the loop range directly in samples (control endpoint, doc
    /// #265 — the GUI drags via the loop-drag gesture messages).
    /// `enabled: None` leaves the loop toggle unchanged. Undoable like
    /// [`Self::ToggleLoop`].
    SetLoopRange {
        loop_in: u64,
        loop_out: u64,
        enabled: Option<bool>,
    },
    StartLoopDrag(LoopDragTarget),
    UpdateLoopDrag(f32),
    EndLoopDrag,
}

impl TransportMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            Self::StartLoopDrag(_) => UndoAction::Begin,
            Self::EndLoopDrag => UndoAction::Commit,
            Self::UpdateLoopDrag(_) => UndoAction::Skip,
            Self::Play
            | Self::Record
            | Self::Pause
            | Self::Stop
            | Self::TogglePlay
            | Self::PlayFromLoopStart
            | Self::SeekTo(_)
            | Self::SkipBack
            | Self::SkipForward
            | Self::SeekToSample(_)
            | Self::SetBpmText(_) => UndoAction::Skip,
            Self::CommitBpm
            | Self::ToggleMetronome
            | Self::CycleTimeSignature
            // Direct control-endpoint setters (doc #265): undoable like
            // their GUI counterparts (cycle / loop toggle+drag).
            | Self::SetTimeSignature { .. }
            | Self::SetLoopRange { .. }
            | Self::SetLoopPoint { .. }
            | Self::ToggleLoop => UndoAction::Record,
        }
    }
}

pub fn handle(r: &mut Resonance, m: TransportMessage) -> Task<Message> {
    match m {
        TransportMessage::TogglePlay => {
            if r.transport.playing {
                // Stop (which also ends a recording pass) and return to
                // where playback started.
                let back = r.transport.play_start;
                let _ = r.engine.send(AudioCommand::Stop);
                r.transport.playing = false;
                r.transport.record_pending = false;
                let _ = r.engine.send(AudioCommand::SeekTo(back));
                r.transport.playhead = back;
            } else {
                return handle(r, TransportMessage::Play);
            }
        }
        TransportMessage::PlayFromLoopStart => {
            // The user asked for the loop start, so it wins over Play's
            // Compose auto-loop of the selected section; and it is where a
            // later Space stop returns to, whether or not we were playing.
            let start = r.transport.loop_in;
            let _ = r.engine.send(AudioCommand::SeekTo(start));
            r.transport.playhead = start;
            r.transport.play_start = start;
            if !r.transport.playing {
                r.session.undo.break_coalesce();
                let _ = r.engine.send(AudioCommand::Play);
                r.transport.playing = true;
            }
        }
        TransportMessage::SeekTo(target) => {
            return super::transport_nav::seek_to(r, target);
        }
        TransportMessage::SetLoopPoint { edge } => {
            return super::transport_nav::set_loop_point(r, edge);
        }
        TransportMessage::Play => {
            // A fresh Play (not already playing) starts a new live-MIDI
            // capture run (FU-D6a): break any coalesce run so a capture in
            // this run doesn't merge into one from an earlier run (or a
            // preceding Record session — `RecordingStarted` breaks that
            // boundary on its own echo, but plain Play has no such echo).
            // Guarded on the transition so a redundant Play dispatched
            // while already playing (e.g. a duplicate control call) does
            // not split an in-progress capture into two entries.
            if !r.transport.playing {
                r.session.undo.break_coalesce();
            }
            // In Compose mode with a selected section, auto-loop that section
            if r.ui.view_mode == ViewMode::Compose {
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
                    let _ = r.engine.send(AudioCommand::SetLoopRange {
                        enabled: true,
                        loop_in,
                        loop_out,
                    });
                    let _ = r.engine.send(AudioCommand::SeekTo(loop_in));
                    r.transport.playhead = loop_in;
                }
            }
            // Where a later Play / Stop toggle returns to. Recorded after
            // the Compose auto-loop above moved the playhead.
            if !r.transport.playing {
                r.transport.play_start = r.transport.playhead;
            }
            let _ = r.engine.send(AudioCommand::Play);
            r.transport.playing = true;
        }
        TransportMessage::Record => {
            if r.registry.tracks.iter().any(|t| t.record_armed) {
                r.transport.record_pending = true;
                if !r.transport.playing {
                    r.transport.play_start = r.transport.playhead;
                }
                let _ = r.engine.send(AudioCommand::Record {
                    precount_bars: r.transport.precount_bars,
                });
                r.transport.playing = true;
            }
        }
        TransportMessage::Pause => {
            let _ = r.engine.send(AudioCommand::Pause);
            r.transport.playing = false;
            r.transport.record_pending = false;
        }
        TransportMessage::Stop => {
            let _ = r.engine.send(AudioCommand::Stop);
            r.transport.playing = false;
            r.transport.record_pending = false;
            r.transport.playhead = 0;
        }
        TransportMessage::SkipBack => {
            let skip = r.sample_rate as u64 * 5;
            let new_pos = r.transport.playhead.saturating_sub(skip);
            let _ = r.engine.send(AudioCommand::SeekTo(new_pos));
            r.transport.playhead = new_pos;
        }
        TransportMessage::SkipForward => {
            let skip = r.sample_rate as u64 * 5;
            let new_pos = r.transport.playhead + skip;
            let _ = r.engine.send(AudioCommand::SeekTo(new_pos));
            r.transport.playhead = new_pos;
        }
        TransportMessage::SeekToSample(pos) => {
            let _ = r.engine.send(AudioCommand::SeekTo(pos));
            r.transport.playhead = pos;
        }
        TransportMessage::SetBpmText(s) => {
            r.transport.bpm_input = s;
            r.transport.bpm_editing = true;
        }
        TransportMessage::CommitBpm => {
            r.transport.bpm_editing = false;
            // `"nan".parse()` is `Ok(NaN)`, and NaN survives `clamp` —
            // it must be rejected here, before it reaches the bar table
            // and re-anchors every clip to sample 0 (review VIEW-06).
            let parsed = r.transport.bpm_input.trim().parse::<f32>().ok();
            if let Some(bpm) = parsed.and_then(resonance_audio::types::sanitize_bpm) {
                // Read every timeline position as a MUSICAL position
                // BEFORE the grid moves, so it can be put back on the same
                // bar afterwards (ba doc #275 P1.4). Only a real tempo
                // change needs it — but the rebuild below runs either way,
                // because committing the value already showing is how a
                // fresh project gets its first bar table.
                let anchors = (bpm != r.transport.bpm).then(|| musical_anchors(r));
                r.transport.bpm = bpm;
                let _ = r.engine.send(AudioCommand::SetBpm { bpm });
                if let Some(first) = r.tempo_events.first_mut() {
                    if first.bar == 0 {
                        first.bpm = bpm;
                    }
                }
                r.rebuild_and_send_tempo();
                if let Some(anchors) = anchors {
                    reanchor_to_tempo(r, anchors);
                }
            }
            r.transport.bpm_input = format!("{:.1}", r.transport.bpm);
        }
        TransportMessage::ToggleMetronome => {
            r.transport.metronome_enabled = !r.transport.metronome_enabled;
            let _ = r.engine.send(AudioCommand::SetMetronomeEnabled {
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
            let _ = r.engine.send(AudioCommand::SetTimeSignature {
                numerator: num,
                denominator: den,
            });
            if let Some(first) = r.signature_events.first_mut() {
                if first.bar == 0 {
                    first.numerator = num;
                    first.denominator = den;
                }
            }
            // Rebuild the GUI-side tempo map so the signature lane's
            // pill marker text + the bar table reflect the new
            // numerator/denominator, and resend the full event list to
            // the engine. Without this the canvas keeps showing the
            // pre-cycle signature on the global-tracks shelf even
            // though the transport bar updated.
            r.rebuild_and_send_tempo();
        }
        TransportMessage::SetTimeSignature {
            numerator,
            denominator,
        } => {
            r.transport.time_sig_num = numerator;
            r.transport.time_sig_den = denominator;
            let _ = r.engine.send(AudioCommand::SetTimeSignature {
                numerator,
                denominator,
            });
            if let Some(first) = r.signature_events.first_mut() {
                if first.bar == 0 {
                    first.numerator = numerator;
                    first.denominator = denominator;
                }
            }
            // Same tempo-map rebuild + resend as CycleTimeSignature so
            // the signature lane and bar table stay in sync.
            r.rebuild_and_send_tempo();
        }
        TransportMessage::SetLoopRange {
            loop_in,
            loop_out,
            enabled,
        } => {
            r.transport.loop_in = loop_in.min(loop_out);
            r.transport.loop_out = loop_in.max(loop_out);
            r.transport.loop_range_set = true;
            if let Some(enabled) = enabled {
                r.transport.loop_enabled = enabled;
            }
            let _ = r.engine.send(AudioCommand::SetLoopRange {
                enabled: r.transport.loop_enabled,
                loop_in: r.transport.loop_in,
                loop_out: r.transport.loop_out,
            });
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
            let _ = r.engine.send(AudioCommand::SetLoopRange {
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
                // Pointer x is in canvas content space (review VIEW-10).
                let seconds = x / r.viewport.zoom;
                let raw = (seconds.max(0.0) as f64 * r.sample_rate as f64) as u64;
                let sample = crate::view::timeline::snap_sample_to_grid_tempo(
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
                    let _ = r.engine.send(AudioCommand::SetLoopRange {
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
                let _ = r.engine.send(AudioCommand::SetLoopRange {
                    enabled: true,
                    loop_in: r.transport.loop_in,
                    loop_out: r.transport.loop_out,
                });
            }
        }
    }
    Task::none()
}
