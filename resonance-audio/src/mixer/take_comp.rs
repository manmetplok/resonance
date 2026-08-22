//! Take-comp playback: render the comped cover of a take group's loop
//! slot by switching the source audio buffer per [`CompSegment`], blending
//! adjacent segments with a short equal-power crossfade so the seam where
//! one take hands off to the next is click-free (design doc #165, epic #15,
//! todo #409).
//!
//! The control thread keeps the authoritative `TakeGroup`s and, on every
//! capture / comp edit / active-take change, flattens them into a
//! [`CompRenderTable`] published wait-free via `ArcSwap`. The audio
//! callback and the offline bounce both load that table into
//! [`BlockInputs::take_comp`](super::render_core::BlockInputs) and reach
//! this module through the shared `render_block`, so realtime playback and
//! a bounced/exported WAV render the comp identically.
//!
//! Two pieces make the comp audible:
//! - [`CompRenderTable::is_governed`] tells the clip phase
//!   ([`super::render::clips::mix_track_clips_governed`]) which recorded
//!   take clips to *skip*, so the raw, overlapping passes never play on
//!   top of the comp.
//! - [`mix_track_comp`] then renders the resolved per-segment spans, each
//!   reading its take's clip, with the equal-power crossfade at every seam.
//!
//! Allocation-free on the audio thread: the table is built (allocating) on
//! the control thread and only *read* here.

use std::collections::HashMap;

use resonance_common::{TakeContent, TakeGroup, TakeGroupId, TimelineRange};

use super::render::clips::CLIP_DECLICK_FRAMES;
use crate::types::{AudioClip, ClipId, FadeCurve, TrackId};

/// Total length, in sample frames, of the equal-power crossfade applied at
/// each comp seam. ~5.3 ms at 48 kHz — long enough to mask the
/// discontinuity between two takes, short enough not to smear transients.
/// Each side of a seam fades over half this window.
pub const COMP_XFADE_FRAMES: u64 = 256;

/// One resolved span of a track's comp: the take clip that is audible over
/// `range`. Adjacent spans always reference distinct clips (the comp helper
/// merges same-take neighbours), so every internal boundary is a real seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompSpan {
    /// Timeline range, in sample frames, this span covers.
    pub range: TimelineRange,
    /// The recorded take clip audible over `range`.
    pub clip_id: ClipId,
}

/// The resolved comp spans for a single track. `spans` is sorted ascending
/// by `range.start` and non-overlapping.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackComp {
    /// The track these spans render onto.
    pub track_id: TrackId,
    /// Ordered, non-overlapping spans (one take clip each).
    pub spans: Vec<CompSpan>,
}

/// A flattened, audio-thread-friendly view of every take group's playback
/// plan. Rebuilt on the control thread whenever a group changes and
/// published wait-free; read (never mutated) by the render path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompRenderTable {
    /// Every recorded take clip under comp control, sorted ascending. These
    /// are skipped on the normal clip-mixing path so only the comp plays.
    governed_clips: Vec<ClipId>,
    /// Per-track resolved spans, sorted ascending by `track_id`.
    tracks: Vec<TrackComp>,
}

impl CompRenderTable {
    /// True when `clip_id` belongs to a take group and must not play on the
    /// ordinary clip path (it is rendered, if selected, by [`mix_track_comp`]).
    #[inline]
    pub fn is_governed(&self, clip_id: ClipId) -> bool {
        self.governed_clips.binary_search(&clip_id).is_ok()
    }

    /// Whether any clip is currently governed — lets the render path skip
    /// the comp work entirely for the common (no take groups) case.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.governed_clips.is_empty()
    }

    /// The resolved comp spans for `track_id`, if the track hosts one.
    #[inline]
    pub fn track_comp(&self, track_id: TrackId) -> Option<&TrackComp> {
        self.tracks
            .binary_search_by_key(&track_id, |t| t.track_id)
            .ok()
            .map(|idx| &self.tracks[idx])
    }
}

/// Flatten the authoritative take groups into a [`CompRenderTable`].
///
/// For each group every recorded *audio* take clip is marked governed (so
/// no raw pass leaks onto the normal clip path), then the audible spans are
/// resolved: an active take solos its whole-slot clip; otherwise the comp's
/// segments map take→clip; failing both, the most recent audio take covers
/// the slot as a sensible default until the app sets a selection. MIDI
/// takes carry no audio clip, so MIDI-only groups contribute nothing here.
///
/// Runs on the control thread (allocation is fine); the result is published
/// to the audio thread via `ArcSwap`.
pub fn build_comp_table(groups: &HashMap<TakeGroupId, TakeGroup>) -> CompRenderTable {
    let mut governed: Vec<ClipId> = Vec::new();
    let mut by_track: HashMap<TrackId, Vec<CompSpan>> = HashMap::new();

    for group in groups.values() {
        // Map each take id to its recorded audio clip (MIDI takes excluded)
        // and govern every such clip so the raw passes never double-play.
        let mut clip_of: HashMap<u64, ClipId> = HashMap::new();
        for take in &group.takes {
            if let TakeContent::Audio { clip_ref } = take.content {
                governed.push(clip_ref);
                clip_of.insert(take.id, clip_ref);
            }
        }
        if clip_of.is_empty() {
            continue; // MIDI-only group: nothing to render on the audio path
        }

        let spans = resolve_spans(group, &clip_of);
        if spans.is_empty() {
            continue;
        }
        by_track.entry(group.track_id).or_default().extend(spans);
    }

    governed.sort_unstable();
    governed.dedup();

    let mut tracks: Vec<TrackComp> = by_track
        .into_iter()
        .map(|(track_id, mut spans)| {
            spans.sort_by_key(|s| s.range.start);
            TrackComp { track_id, spans }
        })
        .collect();
    tracks.sort_by_key(|t| t.track_id);

    CompRenderTable {
        governed_clips: governed,
        tracks,
    }
}

/// Resolve the audible spans of one group: active take overrides the comp,
/// the comp maps segment→clip, and an empty selection falls back to the
/// latest audio take covering the whole slot.
fn resolve_spans(group: &TakeGroup, clip_of: &HashMap<u64, ClipId>) -> Vec<CompSpan> {
    // Active take solos its whole-slot clip, overriding the comp.
    if let Some(active) = group.active_take {
        return match clip_of.get(&active) {
            Some(&clip_id) => vec![CompSpan {
                range: group.slot,
                clip_id,
            }],
            // Active take is a MIDI take (or unknown): nothing on the audio path.
            None => Vec::new(),
        };
    }

    // Otherwise follow the comp's ordered segments.
    if !group.comp.segments.is_empty() {
        return group
            .comp
            .segments
            .iter()
            .filter_map(|seg| {
                clip_of.get(&seg.take_id).map(|&clip_id| CompSpan {
                    range: seg.range,
                    clip_id,
                })
            })
            .collect();
    }

    // No selection yet: default to the most recently captured audio take
    // covering the slot, so a freshly recorded group is audible immediately.
    for take in group.takes.iter().rev() {
        if let Some(&clip_id) = clip_of.get(&take.id) {
            return vec![CompSpan {
                range: group.slot,
                clip_id,
            }];
        }
    }
    Vec::new()
}

/// Mix the comped spans for one track into the de-interleaved track buffers
/// for the window `[playhead, playhead + frames)`. Returns whether any span
/// contributed audio.
///
/// Each span reads its take clip over its range; at every internal seam the
/// outgoing and incoming spans overlap by a short window where each is
/// shaped by the equal-power curve (`sin`/`cos`), so their powers sum to
/// unity and the hand-off is click-free. The crossfade half-width at a seam
/// is clamped to half of the shorter neighbouring span so the two sides
/// always agree, preserving the constant-power property even for very short
/// segments.
///
/// The comp's two *outer* edges are not seams — nothing hands over to them
/// — so they get the same anti-click ramp every clip edge gets on the
/// normal path ([`CLIP_DECLICK_FRAMES`]). Without it the comp would splice
/// straight from silence into whatever the take's waveform happened to be
/// doing, which is exactly the step that ramp exists to remove.
///
/// Shared verbatim by the live mixer and the offline bounce (both reach it
/// through [`render_block`](super::render_core::render_block)), so playback
/// and a bounced WAV render the comp identically. Allocation-free.
pub fn mix_track_comp(
    track_comp: &TrackComp,
    clips: &[AudioClip],
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
) -> bool {
    let buf_start = playhead;
    let buf_end = playhead + frames as u64;
    let spans = &track_comp.spans;
    let mut has_audio = false;

    for (i, span) in spans.iter().enumerate() {
        // Crossfade half-widths into the previous / next neighbour. Both
        // sides of a seam derive the same half from the shorter of the two
        // spans, so the outgoing and incoming ramps are exact complements.
        // The comp's outer edges have no neighbour to hand over to and take
        // the declick ramp instead.
        let (in_half, in_declick) = if i > 0 {
            (seam_half(span.range.length, spans[i - 1].range.length), 0)
        } else {
            (0, declick_frames(span.range.length))
        };
        let (out_half, out_declick) = if i + 1 < spans.len() {
            (seam_half(span.range.length, spans[i + 1].range.length), 0)
        } else {
            (0, declick_frames(span.range.length))
        };

        // The span is audible over its range plus the crossfade tails that
        // reach into each neighbour.
        let audible_start = span.range.start.saturating_sub(in_half);
        let audible_end = span.range.end() + out_half;
        let ov_start = buf_start.max(audible_start);
        let ov_end = buf_end.min(audible_end);
        if ov_start >= ov_end {
            continue;
        }

        let Some(clip) = clips.iter().find(|c| c.id == span.clip_id) else {
            continue;
        };
        let clip_start = clip.start_sample;
        // The retuned cache when the take carries vocal-tuning edits, else
        // the original PCM — the same read the normal clip path makes, so a
        // tuned take comps exactly as it plays (todo #358).
        let clip_data = clip.render_frames();
        let gain_lin = if clip.gain_db == 0.0 {
            1.0
        } else {
            10f32.powf(clip.gain_db / 20.0)
        };

        let fade_in_start = span.range.start.saturating_sub(in_half);
        let fade_out_start = span.range.end().saturating_sub(out_half);

        for timeline_frame in ov_start..ov_end {
            let mut coef = gain_lin;
            // Fade in across the seam with the previous span.
            if in_half > 0 && timeline_frame < span.range.start + in_half {
                let t = (timeline_frame - fade_in_start) as f32 / (in_half * 2) as f32;
                coef *= FadeCurve::EqualPower.coefficient(t);
            }
            // Fade out across the seam with the next span.
            if out_half > 0 && timeline_frame >= fade_out_start {
                let t = (timeline_frame - fade_out_start) as f32 / (out_half * 2) as f32;
                coef *= FadeCurve::EqualPower.coefficient(1.0 - t);
            }
            // Anti-click ramps on the comp's outer edges (no neighbour).
            if in_declick > 0 && timeline_frame < span.range.start + in_declick {
                let t = (timeline_frame - span.range.start) as f32 / in_declick as f32;
                coef *= FadeCurve::EqualPower.coefficient(t);
            }
            if out_declick > 0 && timeline_frame + out_declick >= span.range.end() {
                let t = (span.range.end() - 1).saturating_sub(timeline_frame) as f32
                    / out_declick as f32;
                coef *= FadeCurve::EqualPower.coefficient(t);
            }

            // Source frame within the take clip (same mapping as the normal
            // clip path: timeline → clip-relative + non-destructive trim).
            let clip_frame =
                (timeline_frame - clip_start) as usize + clip.trim_start_frames as usize;
            let clip_idx = clip_frame * 2;
            if clip_idx + 1 < clip_data.len() {
                let off = (timeline_frame - buf_start) as usize;
                track_buf_l[off] += clip_data[clip_idx] * coef;
                track_buf_r[off] += clip_data[clip_idx + 1] * coef;
                has_audio = true;
            }
        }
    }

    has_audio
}

/// Crossfade half-width at a seam between two spans of the given lengths.
/// Capped at half the shorter span so neither span's two seam windows can
/// overlap, which keeps the equal-power complement exact.
#[inline]
fn seam_half(len_a: u64, len_b: u64) -> u64 {
    let shorter = len_a.min(len_b);
    (COMP_XFADE_FRAMES / 2).min(shorter / 2)
}

/// Anti-click ramp length at one of the comp's outer edges, capped at half
/// the span so a very short comp's two edge ramps meet at its midpoint
/// rather than compounding into a double attenuation — the same rule the
/// normal clip path applies to a short clip.
#[inline]
fn declick_frames(span_len: u64) -> u64 {
    CLIP_DECLICK_FRAMES.min(span_len / 2)
}
