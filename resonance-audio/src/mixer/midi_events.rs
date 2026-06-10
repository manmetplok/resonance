//! Per-block MIDI note collection: walk the project's MIDI clips, find
//! notes whose start/end fall inside `[playhead, playhead+frames)`,
//! convert their tick positions to absolute samples through the tempo
//! map, and append sample-accurate `PendingNoteEvent`s into the
//! caller-supplied buffer.

pub(crate) use crate::limits::MAX_MIDI_EVENTS_PER_BUFFER;
use crate::types::*;

/// Collect sample-accurate note events from MIDI clips for a given track and buffer range.
/// Converts tick-based note positions to absolute sample positions using the tempo map.
/// `out` must be pre-allocated and is cleared before use. Stops collecting
/// once `MAX_MIDI_EVENTS_PER_BUFFER` is reached to avoid allocation on the
/// real-time thread.
pub(super) fn collect_midi_events(
    midi_clips: &[MidiClip],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    tempo_map: &TempoMap,
    sample_rate: u32,
    out: &mut Vec<PendingNoteEvent>,
) {
    out.clear();
    let buf_end = playhead + frames as u64;

    for clip in midi_clips.iter().filter(|c| c.track_id == track_id) {
        let visible_start = clip.trim_start_ticks;
        let visible_end = clip.duration_ticks.saturating_sub(clip.trim_end_ticks);

        for note in &clip.notes {
            // Skip notes outside the visible (trimmed) range
            if note.start_tick + note.duration_ticks <= visible_start {
                continue;
            }
            if note.start_tick >= visible_end {
                continue;
            }

            // Clamp note start/end to visible range
            let effective_start = note.start_tick.max(visible_start);
            let effective_end = (note.start_tick + note.duration_ticks).min(visible_end);

            // Convert to absolute sample positions using the tempo map
            // so tick→sample accounts for tempo changes across the clip.
            let note_abs_start = tempo_map.tick_to_abs_sample(
                clip.start_sample,
                effective_start - visible_start,
                sample_rate,
            );
            let note_abs_end = tempo_map.tick_to_abs_sample(
                clip.start_sample,
                effective_end - visible_start,
                sample_rate,
            );

            // Emit NoteOn if it falls in this buffer
            if note_abs_start >= playhead && note_abs_start < buf_end {
                if out.len() >= MAX_MIDI_EVENTS_PER_BUFFER {
                    break;
                }
                out.push(PendingNoteEvent {
                    is_note_on: true,
                    note: note.note,
                    velocity: note.velocity,
                    sample_offset: (note_abs_start - playhead) as u32,
                });
            }

            // Emit NoteOff if it falls in this buffer
            if note_abs_end >= playhead && note_abs_end < buf_end {
                if out.len() >= MAX_MIDI_EVENTS_PER_BUFFER {
                    break;
                }
                out.push(PendingNoteEvent {
                    is_note_on: false,
                    note: note.note,
                    velocity: 0.0,
                    sample_offset: (note_abs_end - playhead) as u32,
                });
            }
        }
    }

    // Sort by sample offset for CLAP compliance. Unstable to avoid the
    // stable sort's heap allocation on the audio thread; note-offs are
    // keyed before note-ons so retriggers at the same offset stay paired.
    out.sort_unstable_by_key(|e| (e.sample_offset, e.is_note_on));
}

/// Public version of collect_midi_events for the bounce path. Exposed
/// outside the crate for integration-test access — production callers
/// stay inside `resonance-audio`.
pub fn collect_midi_events_bounce(
    midi_clips: &[MidiClip],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    tempo_map: &TempoMap,
    sample_rate: u32,
    out: &mut Vec<PendingNoteEvent>,
) {
    out.clear();
    collect_midi_events(
        midi_clips,
        track_id,
        playhead,
        frames,
        tempo_map,
        sample_rate,
        out,
    );
}
