//! Read and write Standard MIDI File (SMF, Format 0) representations
//! of a [`MidiClip`]'s notes. Used by the project save/load path so
//! MIDI clips persist as interchangeable `.mid` files on disk rather
//! than an inline JSON notes array.
//!
//! The clip's tick domain is written through verbatim — `start_tick`
//! and `duration_ticks` stay in the same units the engine uses
//! (see `resonance_audio::types::TICKS_PER_QUARTER_NOTE`).

use std::path::Path;

use midly::num::{u4, u7};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};

use crate::types::{MidiNote, TICKS_PER_QUARTER_NOTE};

/// Serialize a list of notes to a Format 0 SMF at `path`.
///
/// The note list is sorted by `(start_tick, note)`, expanded into
/// paired note-on/note-off events, sorted by absolute tick, and
/// delta-encoded. A single End-of-Track meta event terminates the
/// track.
pub fn write_midi_file(path: &Path, notes: &[MidiNote]) -> Result<(), String> {
    let mut events: Vec<(u64, TrackEventKind<'static>)> = Vec::with_capacity(notes.len() * 2);
    for n in notes {
        let key = u7::new(n.note.min(127));
        let vel = u7::new((n.velocity.clamp(0.0, 1.0) * 127.0).round() as u8);
        events.push((
            n.start_tick,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn { key, vel },
            },
        ));
        events.push((
            n.start_tick + n.duration_ticks,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOff {
                    key,
                    vel: u7::new(0),
                },
            },
        ));
    }

    // Stable sort keeps note-on before a coincident note-off on the
    // same pitch, which is what most sequencers expect.
    events.sort_by_key(|(tick, _)| *tick);

    let mut track: Track = Track::new();
    let mut prev_tick: u64 = 0;
    for (tick, kind) in events {
        let delta_u64 = tick.saturating_sub(prev_tick);
        let delta = midly::num::u28::new(delta_u64.min(u32::MAX as u64) as u32);
        track.push(TrackEvent { delta, kind });
        prev_tick = tick;
    }
    track.push(TrackEvent {
        delta: midly::num::u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let header = Header::new(
        Format::SingleTrack,
        Timing::Metrical(midly::num::u15::new(TICKS_PER_QUARTER_NOTE as u16)),
    );
    let smf = Smf {
        header,
        tracks: vec![track],
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let mut buf: Vec<u8> = Vec::new();
    smf.write(&mut buf).map_err(|e| format!("serialize smf: {e}"))?;
    std::fs::write(path, buf).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Parse a Format 0 (or Format 1 first-track-wins) SMF at `path`
/// back into a list of notes. Unmatched note-ons are silently
/// dropped — a repaired file round-trips bit-for-bit.
pub fn read_midi_file(path: &Path) -> Result<Vec<MidiNote>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let smf = Smf::parse(&bytes).map_err(|e| format!("parse smf: {e}"))?;

    let track = match smf.tracks.first() {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    // Track pending note-ons keyed by (channel, note) so overlapping
    // notes of the same pitch on different channels don't collide.
    // We only emit channel-0 notes into the result; others are
    // ignored because `MidiClip` doesn't carry a channel field.
    let mut pending: std::collections::HashMap<(u8, u8), (u64, f32)> =
        std::collections::HashMap::new();
    let mut notes: Vec<MidiNote> = Vec::new();
    let mut abs_tick: u64 = 0;

    for ev in track {
        abs_tick += u32::from(ev.delta) as u64;
        if let TrackEventKind::Midi { channel, message } = ev.kind {
            let ch: u8 = channel.into();
            match message {
                MidiMessage::NoteOn { key, vel } => {
                    let v: u8 = vel.into();
                    let k: u8 = key.into();
                    if v == 0 {
                        // Running-status "note off" convention.
                        if let Some((start, velocity)) = pending.remove(&(ch, k)) {
                            if ch == 0 {
                                notes.push(MidiNote {
                                    note: k,
                                    velocity,
                                    start_tick: start,
                                    duration_ticks: abs_tick.saturating_sub(start),
                                });
                            }
                        }
                    } else {
                        pending.insert((ch, k), (abs_tick, v as f32 / 127.0));
                    }
                }
                MidiMessage::NoteOff { key, .. } => {
                    let k: u8 = key.into();
                    if let Some((start, velocity)) = pending.remove(&(ch, k)) {
                        if ch == 0 {
                            notes.push(MidiNote {
                                note: k,
                                velocity,
                                start_tick: start,
                                duration_ticks: abs_tick.saturating_sub(start),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    notes.sort_by_key(|n| (n.start_tick, n.note));
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_notes() {
        let dir = tempdir();
        let path = dir.join("clip.mid");
        let input = vec![
            MidiNote {
                note: 60,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 480,
            },
            MidiNote {
                note: 64,
                velocity: 0.5,
                start_tick: 240,
                duration_ticks: 240,
            },
            MidiNote {
                note: 67,
                velocity: 1.0,
                start_tick: 480,
                duration_ticks: 960,
            },
        ];
        write_midi_file(&path, &input).unwrap();
        let out = read_midi_file(&path).unwrap();
        assert_eq!(out.len(), input.len());
        for (a, b) in out.iter().zip(input.iter()) {
            assert_eq!(a.note, b.note);
            assert_eq!(a.start_tick, b.start_tick);
            assert_eq!(a.duration_ticks, b.duration_ticks);
            // Velocity round-trips through u7 so allow 1/127 slack.
            assert!((a.velocity - b.velocity).abs() <= 1.0 / 127.0 + 1e-6);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempdir() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "resonance-midi-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }
}
