//! Group a voiced f0 contour into note *blobs* for vocal tuning.
//!
//! This is DSP step 2 of the vocal-tuning epic (#27): it consumes the
//! per-frame [`F0Frame`] contour produced by [`crate::pitch`] and partitions
//! the voiced frames into [`NoteBlob`]s — onset/offset, a mean pitch (as a
//! fractional MIDI note plus its cents deviation from the nearest semitone),
//! and the per-frame cents-deviation contour inside the note. Downstream this
//! drives the pitch-editor canvas (#360) and formant-preserving resynthesis
//! (#353).
//!
//! Two robustness mechanisms keep musical gestures intact:
//!
//! * **Median smoothing** of the MIDI contour rejects single-frame octave
//!   errors and dropouts before any segmentation decision, so a stray glitch
//!   never spawns a spurious note or skews a blob's pitch.
//! * **Hysteresis** on the note boundary: the pitch must move clear of the
//!   current note's band *and* settle at a new semitone for a minimum number of
//!   frames before a new blob opens. Vibrato and portamento therefore stay
//!   inside one blob rather than fragmenting it.
//!
//! Brief unvoiced dips (consonants, glottal stops) up to `max_gap_frames` are
//! bridged so a legato phrase stays whole, while longer gaps end the note so
//! detached phrases segment cleanly.
//!
//! Everything here is pure and deterministic: [`segment_notes`] is a free
//! function over a contour slice and allocates only its output.

use crate::pitch::F0Frame;

/// MIDI note number of a frequency in Hz (A4 = 440 Hz = MIDI 69).
fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// One detected note: a contiguous run of voiced frames at a stable pitch.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteBlob {
    /// Index of the first frame of the blob in the input contour (inclusive).
    pub onset_frame: usize,
    /// Index of the last frame of the blob in the input contour (inclusive).
    pub offset_frame: usize,
    /// Onset time in seconds (the onset frame's `time_secs`).
    pub onset_secs: f32,
    /// Offset time in seconds (the offset frame's `time_secs`).
    pub offset_secs: f32,
    /// Mean pitch over the blob as a fractional MIDI note number.
    pub midi: f32,
    /// Nearest integer MIDI note (`midi` rounded), clamped to `0..=127`.
    pub note: u8,
    /// Mean deviation from [`note`](Self::note), in cents (`(midi − note)·100`).
    /// In `(-50, 50]` for an in-range note.
    pub cents_offset: f32,
    /// Per-frame cents deviation from [`note`](Self::note), one entry per voiced
    /// frame in the blob (in frame order). Captures vibrato/drift inside the
    /// note; its mean is [`cents_offset`](Self::cents_offset).
    pub cents_contour: Vec<f32>,
}

/// Configuration for [`segment_notes`].
///
/// Thresholds expressed in *frames* refer to analysis frames of the contour
/// (one per f0 hop), so they scale with the detector's hop size rather than the
/// sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentConfig {
    /// Median-filter window, in frames, applied to the MIDI contour before
    /// segmentation. Larger rejects longer glitches but blurs fast runs. `0` or
    /// `1` disables smoothing. Odd values are natural; even values still work.
    pub median_window: usize,
    /// Extra half-width, in semitones, added beyond `±0.5` around the current
    /// note before the pitch is considered to have left it. This is the
    /// hysteresis that keeps vibrato inside one blob.
    pub hysteresis_semitones: f32,
    /// A new pitch must persist this many consecutive frames before a new blob
    /// opens. Rejects brief excursions (vibrato peaks, portamento transit).
    pub min_note_frames: usize,
    /// Unvoiced gaps up to this many frames are bridged within a note; longer
    /// gaps end the note.
    pub max_gap_frames: usize,
    /// Blobs with fewer than this many voiced frames are discarded as spurious.
    pub min_blob_frames: usize,
    /// Frames with confidence below this are ignored (in addition to the
    /// frame's own `voiced` flag). `0.0` trusts the detector's flag alone.
    pub min_confidence: f32,
}

impl Default for SegmentConfig {
    /// Vocal-tuned defaults: 5-frame median window, 0.3-semitone hysteresis,
    /// 4-frame note/gap minimums, 3-frame minimum blob.
    fn default() -> Self {
        Self {
            median_window: 5,
            hysteresis_semitones: 0.3,
            min_note_frames: 4,
            max_gap_frames: 4,
            min_blob_frames: 3,
            min_confidence: 0.0,
        }
    }
}

/// Segment a voiced f0 `contour` into [`NoteBlob`]s.
///
/// Frames are usable when `voiced` and `confidence >= config.min_confidence`;
/// all others (unvoiced, silence, low confidence) are treated as gaps. Returns
/// an empty vector when no note survives the minimum-length filter.
pub fn segment_notes(contour: &[F0Frame], config: SegmentConfig) -> Vec<NoteBlob> {
    let n = contour.len();
    if n == 0 {
        return Vec::new();
    }

    // Raw MIDI per usable frame; `None` marks gaps (unvoiced / low confidence).
    let raw_midi: Vec<Option<f32>> = contour
        .iter()
        .map(|f| {
            if f.voiced && f.f0_hz > 0.0 && f.confidence >= config.min_confidence {
                Some(hz_to_midi(f.f0_hz))
            } else {
                None
            }
        })
        .collect();

    // Median-smooth the MIDI contour over usable neighbours only, so gaps never
    // pull silence into the window and a lone octave error is replaced by the
    // local median.
    let smid = median_smooth(&raw_midi, config.median_window);

    // Walk the contour, opening/closing blobs with hysteresis on the note band.
    let mut blobs: Vec<Vec<usize>> = Vec::new();
    let mut cur_level: Option<i32> = None;
    let mut cur_frames: Vec<usize> = Vec::new();
    let mut pending_level: Option<i32> = None;
    let mut pending: Vec<usize> = Vec::new();
    let mut gap = 0usize;

    for (i, &slot) in smid.iter().enumerate() {
        let Some(m) = slot else {
            // Unusable frame: count toward the gap; close the note once the gap
            // outlasts a bridgeable dip.
            if cur_level.is_some() {
                gap += 1;
                if gap > config.max_gap_frames {
                    cur_frames.append(&mut pending);
                    pending_level = None;
                    blobs.push(std::mem::take(&mut cur_frames));
                    cur_level = None;
                    gap = 0;
                }
            }
            continue;
        };
        gap = 0;

        let Some(lvl) = cur_level else {
            // First voiced frame opens the first note at its nearest semitone.
            cur_level = Some(m.round() as i32);
            cur_frames.push(i);
            continue;
        };

        let half = 0.5 + config.hysteresis_semitones;
        if (m - lvl as f32).abs() <= half {
            // Still inside the current note: any tentative new note was a brief
            // excursion, so fold its frames back in.
            cur_frames.append(&mut pending);
            pending_level = None;
            cur_frames.push(i);
        } else {
            // Outside the band: a candidate new note must persist to commit.
            let cand = m.round() as i32;
            if pending_level != Some(cand) {
                // Pitch is in transit (e.g. portamento): the old tentative
                // frames belong to the transition, not a new note.
                cur_frames.append(&mut pending);
                pending_level = Some(cand);
            }
            pending.push(i);
            if pending.len() >= config.min_note_frames {
                blobs.push(std::mem::take(&mut cur_frames));
                cur_frames = std::mem::take(&mut pending);
                cur_level = Some(cand);
                pending_level = None;
            }
        }
    }
    // Flush whatever note is still open at the end.
    if cur_level.is_some() {
        cur_frames.append(&mut pending);
        blobs.push(std::mem::take(&mut cur_frames));
    }

    blobs
        .into_iter()
        .filter(|idxs| idxs.len() >= config.min_blob_frames.max(1))
        .map(|idxs| build_blob(contour, &smid, &idxs))
        .collect()
}

/// Median-filter `values` over a window of `window` samples, ignoring `None`
/// entries (gaps) so the filter only ever averages usable neighbours. Gap
/// positions stay `None`.
fn median_smooth(values: &[Option<f32>], window: usize) -> Vec<Option<f32>> {
    let n = values.len();
    if window <= 1 {
        return values.to_vec();
    }
    let r = window / 2;
    let mut buf: Vec<f32> = Vec::with_capacity(window);
    (0..n)
        .map(|i| {
            values[i]?;
            buf.clear();
            let lo = i.saturating_sub(r);
            let hi = (i + r).min(n - 1);
            for v in values[lo..=hi].iter().flatten() {
                buf.push(*v);
            }
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap());
            Some(buf[buf.len() / 2])
        })
        .collect()
}

/// Build a [`NoteBlob`] from the (ascending) frame indices of one note, using
/// the smoothed MIDI values so glitches do not skew the pitch statistics.
fn build_blob(contour: &[F0Frame], smid: &[Option<f32>], idxs: &[usize]) -> NoteBlob {
    let midi_vals: Vec<f32> = idxs.iter().map(|&i| smid[i].unwrap()).collect();
    let mean_midi = midi_vals.iter().sum::<f32>() / midi_vals.len() as f32;
    let note = (mean_midi.round() as i32).clamp(0, 127) as u8;
    let cents_offset = (mean_midi - note as f32) * 100.0;
    let cents_contour = midi_vals
        .iter()
        .map(|&m| (m - note as f32) * 100.0)
        .collect();

    let onset_frame = idxs[0];
    let offset_frame = idxs[idxs.len() - 1];
    NoteBlob {
        onset_frame,
        offset_frame,
        onset_secs: contour[onset_frame].time_secs,
        offset_secs: contour[offset_frame].time_secs,
        midi: mean_midi,
        note,
        cents_offset,
        cents_contour,
    }
}
