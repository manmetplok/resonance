//! Update handlers for the global chord track (todo #441, doc #168).
//!
//! Routed from `update::dispatch` like [`global_track`](super::global_track).
//! Each mutating message is classified `Record` in
//! [`crate::undo::classify`], so every edit here pushes exactly one undo
//! entry. The chord track itself lives in app state (see
//! [`crate::chord_track`]) and is carried through undo via
//! [`crate::undo::UndoExtras`]; nothing here touches the realtime engine.
//!
//! Positions arrive as raw sample positions and are snapped to the
//! timeline grid with the same helper the clip-drag handlers use, so
//! chord edits land on the same bar/beat lines as everything else.
//! `MoveStart`/`SetEnd` additionally clamp against the neighbouring
//! regions to keep `regions` sorted and non-overlapping.

use iced::Task;
use resonance_music_theory::{parse_chord, Chord, ChordQuality, PitchClass};

use crate::chord_track::{ChordRegion, KeyChange};
use crate::message::{ChordTrackMessage, Message};
use crate::view::timeline::snap_sample_to_grid_tempo;
use crate::Resonance;

impl Resonance {
    /// Snap a raw sample position to the timeline grid using the current
    /// tempo map and zoom — the shared clip/transport snap.
    fn chord_snap(&self, sample: u64) -> u64 {
        snap_sample_to_grid_tempo(
            sample,
            self.transport.bpm,
            self.transport.time_sig_num,
            self.sample_rate,
            self.viewport.zoom,
            &self.tempo_map,
        )
    }

    /// One bar in samples at the current tempo/signature — the default
    /// length for a region added with no following region to abut.
    fn chord_default_bar_len(&self) -> u64 {
        let bpm = self.transport.bpm.max(1.0) as f64;
        let samples_per_beat = self.sample_rate as f64 * 60.0 / bpm;
        (samples_per_beat * self.transport.time_sig_num.max(1) as f64).round() as u64
    }
}

pub fn handle(r: &mut Resonance, m: ChordTrackMessage) -> Task<Message> {
    match m {
        ChordTrackMessage::AddAtPlayhead => {
            let start = r.chord_snap(r.transport.playhead);
            // Never stack two regions at the same start.
            if r.chord_track.regions.iter().any(|rg| rg.start_sample == start) {
                return Task::none();
            }
            // If the playhead lands inside a region, split it: the host
            // region ends at `start` and the new region inherits its tail.
            let split_tail = r
                .chord_track
                .regions
                .iter_mut()
                .find(|rg| rg.start_sample < start && start < rg.end_sample)
                .map(|rg| {
                    let old_end = rg.end_sample;
                    rg.end_sample = start;
                    old_end
                });
            let end = match split_tail {
                Some(tail_end) => tail_end,
                None => {
                    // Free space: default one bar, capped by the next region.
                    let next = r
                        .chord_track
                        .regions
                        .iter()
                        .map(|rg| rg.start_sample)
                        .filter(|&s| s > start)
                        .min();
                    let default_end = start + r.chord_default_bar_len();
                    next.map_or(default_end, |n| default_end.min(n))
                }
            };
            if end <= start {
                return Task::none();
            }
            let id = r.compose.fresh_id();
            r.chord_track.last_error = None;
            r.chord_track.insert_region(ChordRegion {
                id,
                chord: Chord::new(PitchClass::C, ChordQuality::Maj),
                start_sample: start,
                end_sample: end,
                pinned: false,
            });
        }

        ChordTrackMessage::AddRegion {
            start_sample,
            end_sample,
            symbol,
        } => {
            let chord = match parse_chord(&symbol) {
                Ok(c) => c,
                Err(e) => {
                    r.chord_track.last_error = Some(e.to_string());
                    return Task::none();
                }
            };
            let start = r.chord_snap(start_sample);
            let mut end = r.chord_snap(end_sample);
            // Don't overlap the next region forward.
            if let Some(next) = r
                .chord_track
                .regions
                .iter()
                .map(|rg| rg.start_sample)
                .filter(|&s| s > start)
                .min()
            {
                end = end.min(next);
            }
            if end <= start {
                r.chord_track.last_error = Some("chord region has no length".to_string());
                return Task::none();
            }
            let id = r.compose.fresh_id();
            r.chord_track.last_error = None;
            r.chord_track.insert_region(ChordRegion {
                id,
                chord,
                start_sample: start,
                end_sample: end,
                pinned: false,
            });
        }

        ChordTrackMessage::SetSymbol { id, symbol } => match parse_chord(&symbol) {
            Ok(chord) => {
                let applied = if let Some(region) = r.chord_track.region_mut(id) {
                    region.chord = chord;
                    true
                } else {
                    false
                };
                if applied {
                    r.chord_track.last_error = None;
                }
            }
            Err(e) => {
                r.chord_track.last_error = Some(e.to_string());
            }
        },

        ChordTrackMessage::MoveStart { id, sample } => {
            let snapped = r.chord_snap(sample);
            let Some(idx) = r.chord_track.regions.iter().position(|rg| rg.id == id) else {
                return Task::none();
            };
            let own_end = r.chord_track.regions[idx].end_sample;
            let prev_end = if idx > 0 {
                r.chord_track.regions[idx - 1].end_sample
            } else {
                0
            };
            // Stay after the previous region and strictly before own end.
            let new_start = snapped.max(prev_end).min(own_end.saturating_sub(1));
            r.chord_track.regions[idx].start_sample = new_start;
            r.chord_track.resort();
        }

        ChordTrackMessage::SetEnd { id, sample } => {
            let snapped = r.chord_snap(sample);
            let Some(idx) = r.chord_track.regions.iter().position(|rg| rg.id == id) else {
                return Task::none();
            };
            let own_start = r.chord_track.regions[idx].start_sample;
            let next_start = r
                .chord_track
                .regions
                .get(idx + 1)
                .map(|rg| rg.start_sample)
                .unwrap_or(u64::MAX);
            // Stay strictly after own start and at/before the next region.
            let new_end = snapped.max(own_start + 1).min(next_start);
            r.chord_track.regions[idx].end_sample = new_end;
        }

        ChordTrackMessage::Delete { id } => {
            r.chord_track.remove_region(id);
        }

        ChordTrackMessage::TogglePin { id } => {
            if let Some(region) = r.chord_track.region_mut(id) {
                region.pinned = !region.pinned;
            }
        }

        ChordTrackMessage::SetSongKey { scale } => {
            if let Some(first) = r.chord_track.key_changes.first_mut() {
                first.scale = scale;
            } else {
                let id = r.compose.fresh_id();
                r.chord_track.insert_key_change(KeyChange {
                    id,
                    start_sample: 0,
                    scale,
                });
            }
        }

        ChordTrackMessage::InsertKeyChange { sample, scale } => {
            let snapped = r.chord_snap(sample);
            if let Some(existing) = r
                .chord_track
                .key_changes
                .iter_mut()
                .find(|k| k.start_sample == snapped)
            {
                existing.scale = scale;
            } else {
                let id = r.compose.fresh_id();
                r.chord_track.insert_key_change(KeyChange {
                    id,
                    start_sample: snapped,
                    scale,
                });
            }
        }

        ChordTrackMessage::MoveKeyChange { id, sample } => {
            let snapped = r.chord_snap(sample);
            let moved = if let Some(kc) = r.chord_track.key_change_mut(id) {
                kc.start_sample = snapped;
                true
            } else {
                false
            };
            if moved {
                r.chord_track.resort();
            }
        }

        ChordTrackMessage::DeleteKeyChange { id } => {
            r.chord_track.remove_key_change(id);
        }
    }
    Task::none()
}
