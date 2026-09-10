//! Per-block MIDI note collection: walk the project's MIDI clips, find
//! notes whose start/end fall inside `[playhead, playhead+frames)`,
//! convert their tick positions to absolute samples through the tempo
//! map, and append sample-accurate `PendingNoteEvent`s into the
//! caller-supplied buffer.
//!
//! ## Per-block windowing
//!
//! The tick→sample conversion ([`TempoMap::tick_to_abs_sample`]) costs two
//! binary searches over the bar table plus float work, and a block only
//! ever contains a couple of note events. Running it for every note of
//! every clip made this function `O(notes in the whole arrangement)` per
//! block *per instrument track* — on an arrangement-length project that
//! dominated the entire realtime budget (measured: 728 µs of a 2.67 ms
//! quantum for 6 tracks × 4096 notes, with no plugins running at all).
//!
//! So each clip first derives a **tick window** for the block and rejects
//! notes with two integer comparisons before any conversion runs. The
//! bounds are obtained from the inverse mapping and then *verified* with a
//! single forward conversion each: if the inverse rounded the wrong way
//! the bound degrades to "no filtering" rather than to a dropped note.
//! Surviving notes take exactly the original code path, so the emitted
//! event list is byte-identical.

pub(crate) use crate::limits::MAX_MIDI_EVENTS_PER_BUFFER;
use crate::types::*;

/// Ticks of slack applied to the inverse (sample→tick) estimate before it
/// is verified as a window bound. Absorbs the rounding difference between
/// the forward and inverse bar-table mappings; a too-large value only
/// costs a few extra conversions, never correctness.
const WINDOW_SLACK_TICKS: u64 = 64;

/// Samples of slack demanded when verifying a window bound, so a bound is
/// only accepted when it is *strictly* outside the block by more than any
/// sub-sample rounding in the forward mapping.
const WINDOW_VERIFY_SAMPLES: u64 = 2;

/// Minimum note count for a clip to be worth deriving a tick window for.
/// Setting the window up costs four tempo-map conversions; the per-note
/// saving is roughly one conversion, so anything above a handful of notes
/// wins and a two-note clip is left alone.
const WINDOW_MIN_NOTES: usize = 8;

/// Tick-offset window (relative to a clip's visible start) outside which
/// no note of that clip can emit an event into `[playhead, buf_end)`.
struct TickWindow {
    /// A note whose effective **end** tick offset is `<= lo` cannot fire.
    lo: u64,
    /// A note whose effective **start** tick offset is `> hi` cannot fire.
    hi: u64,
}

/// Derive the per-block tick window for one clip. Both bounds are sound
/// by construction: each is checked with one forward conversion and falls
/// back to the unfiltered extreme (`0` / `u64::MAX`) when the check fails.
fn clip_tick_window(
    clip: &MidiClip,
    tempo_map: &TempoMap,
    sample_rate: u32,
    playhead: u64,
    buf_end: u64,
) -> TickWindow {
    let clip_base_tick = tempo_map.sample_to_abs_tick(clip.start_sample, sample_rate);
    let lo_est = tempo_map
        .sample_to_abs_tick(playhead, sample_rate)
        .saturating_sub(clip_base_tick);
    let hi_est = tempo_map
        .sample_to_abs_tick(buf_end, sample_rate)
        .saturating_sub(clip_base_tick);

    let lo_candidate = lo_est.saturating_sub(WINDOW_SLACK_TICKS);
    let lo = if lo_candidate > 0
        && tempo_map.tick_to_abs_sample(clip.start_sample, lo_candidate, sample_rate)
            + WINDOW_VERIFY_SAMPLES
            <= playhead
    {
        lo_candidate
    } else {
        0
    };

    let hi_candidate = hi_est.saturating_add(WINDOW_SLACK_TICKS);
    let hi = if tempo_map.tick_to_abs_sample(clip.start_sample, hi_candidate, sample_rate)
        >= buf_end + WINDOW_VERIFY_SAMPLES
    {
        hi_candidate
    } else {
        u64::MAX
    };

    TickWindow { lo, hi }
}

/// Collect sample-accurate note events from MIDI clips for a given track and buffer range.
/// Converts tick-based note positions to absolute sample positions using the tempo map.
/// `out` must be pre-allocated and is cleared before use. Never grows past
/// `MAX_MIDI_EVENTS_PER_BUFFER` (no allocation on the real-time thread);
/// at the cap note-offs take priority over note-ons — see [`push_capped`].
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
    // Note-ons currently in `out`, kept so the cap's eviction path knows
    // without a scan whether there is anything left to evict.
    let mut note_ons_queued: usize = 0;

    for clip in midi_clips.iter().filter(|c| c.track_id == track_id) {
        let visible_start = clip.trim_start_ticks;
        let visible_end = clip.duration_ticks.saturating_sub(clip.trim_end_ticks);
        if visible_end <= visible_start {
            continue;
        }

        // Clip-level rejection. Every event this clip can emit sits in
        // `[clip.start_sample, clip_end_sample]` (note ticks are clamped
        // to the visible range before conversion), so a clip that does
        // not touch the block is skipped without looking at its notes.
        // Both ends carry the same sub-sample rounding margin the tick
        // bounds use.
        if buf_end + WINDOW_VERIFY_SAMPLES <= clip.start_sample {
            continue;
        }
        let clip_end_sample = tempo_map.tick_to_abs_sample(
            clip.start_sample,
            visible_end - visible_start,
            sample_rate,
        );
        if playhead > clip_end_sample + WINDOW_VERIFY_SAMPLES {
            continue;
        }

        // Note-level tick window, in raw `start_tick` coordinates so the
        // per-note test is two integer comparisons and one add. Deriving
        // it costs four tempo-map conversions, so a clip too small to
        // amortise them keeps the unfiltered range.
        let (lo_abs, hi_abs) = if clip.notes.len() >= WINDOW_MIN_NOTES {
            let window = clip_tick_window(clip, tempo_map, sample_rate, playhead, buf_end);
            (
                window.lo.saturating_add(visible_start),
                window.hi.saturating_add(visible_start),
            )
        } else {
            (0, u64::MAX)
        };

        for note in &clip.notes {
            let note_end_tick = note.start_tick.saturating_add(note.duration_ticks);
            // Window reject: the note ends before the block starts, or
            // starts after the block ends. Sound because
            // `tick_to_abs_sample` is monotonic in the tick offset and
            // both bounds were verified above.
            if note_end_tick <= lo_abs || note.start_tick > hi_abs {
                continue;
            }
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
                push_capped(
                    out,
                    &mut note_ons_queued,
                    PendingNoteEvent {
                        is_note_on: true,
                        note: note.note,
                        velocity: note.velocity,
                        sample_offset: (note_abs_start - playhead) as u32,
                    },
                );
            }

            // Emit NoteOff if it falls in this buffer
            if note_abs_end >= playhead && note_abs_end < buf_end {
                push_capped(
                    out,
                    &mut note_ons_queued,
                    PendingNoteEvent {
                        is_note_on: false,
                        note: note.note,
                        velocity: 0.0,
                        sample_offset: (note_abs_end - playhead) as u32,
                    },
                );
            }
        }
    }

    // Sort by sample offset for CLAP compliance. Unstable to avoid the
    // stable sort's heap allocation on the audio thread; note-offs are
    // keyed before note-ons so retriggers at the same offset stay paired.
    out.sort_unstable_by_key(|e| (e.sample_offset, e.is_note_on));
}

/// Append `event` to `out` without ever exceeding
/// `MAX_MIDI_EVENTS_PER_BUFFER`. Past the cap a note-on is dropped (a
/// note that never sounds is harmless), while a note-off evicts a queued
/// note-on to make room — mirroring `MidiStash::stash` — so a voice whose
/// note-on already reached the plugin in an earlier block still receives
/// its release instead of sticking. If the buffer holds nothing but
/// note-offs the incoming one is dropped; its counterpart note-on was
/// itself dropped or evicted at this cap in the same or an earlier block.
///
/// Audio-thread safe: in-place on the pre-allocated buffer, and bounded —
/// `note_ons` (the count of note-ons currently in `out`, maintained here)
/// gates the eviction scan, and note-ons past the cap never enter the
/// buffer, so at most `MAX_MIDI_EVENTS_PER_BUFFER` scans happen per block
/// however many notes overflow.
fn push_capped(out: &mut Vec<PendingNoteEvent>, note_ons: &mut usize, event: PendingNoteEvent) {
    if out.len() < MAX_MIDI_EVENTS_PER_BUFFER {
        if event.is_note_on {
            *note_ons += 1;
        }
        out.push(event);
        return;
    }
    if event.is_note_on || *note_ons == 0 {
        return;
    }
    if let Some(idx) = out.iter().position(|e| e.is_note_on) {
        out.remove(idx);
        out.push(event);
        *note_ons -= 1;
    }
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
