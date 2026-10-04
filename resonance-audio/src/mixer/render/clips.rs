//! The audio-clip half of a track's source signal: the per-block clip
//! mix, its fade-in / fade-out envelopes, the automatic same-track
//! crossfade, the anti-click edge ramp, and the recorded-playback monitor
//! gate that decides whether the live input joins them.

use std::borrow::Borrow;

use crate::mixer::take_comp::CompRenderTable;
use crate::types::*;

/// Recorded-playback monitor gate (doc #257): `true` when the track's
/// live input monitor must be *skipped* for the block `[playhead,
/// playhead + frames)` because the track's playback source is
/// `Recorded` and a recorded take (an audio clip on the track) covers
/// the block — the take plays through the normal clip mix and the
/// hardware return must not be layered on top of it.
///
/// A record-armed track is never gated, so a punch-in still monitors
/// the hardware while re-recording over an existing take. The stopped
/// transport never reaches this path at all (`monitor.rs`'s
/// passthrough handles stopped monitoring), so monitoring while
/// preparing a take is untouched. Block granularity matches the
/// monitor stream itself (~a few ms). Cheap: two relaxed atomic loads,
/// and the `O(clips)` span scan only runs for `Recorded` tracks.
pub fn recorded_monitor_gate<C: Borrow<AudioClip>>(
    track: &Track,
    clips: &[C],
    playhead: u64,
    frames: usize,
) -> bool {
    track.playback_source() == resonance_common::PlaybackSource::Recorded
        && !track.record_armed()
        && audio_clip_covers(clips, track.id, playhead, playhead + frames as u64)
}

/// Mix every audio clip on `track_id` into the de-interleaved track
/// buffers for the timeline window `[playhead, playhead + frames)`,
/// applying per-frame the single coefficient
/// `fade_in_envelope × fade_out_envelope × dB→linear(gain_db)`. Returns
/// whether any clip contributed audio.
///
/// Where two clips on the same track overlap, the overlap region is an
/// automatic crossfade: the earlier clip fades out and the later clip
/// fades in across the shared span. With the default equal-power curves
/// the two contributions sum to constant power, so the seam is
/// click-free. An explicit fade that is longer than the overlap reshapes
/// the crossfade (the longer of the two lengths wins).
///
/// Edges that no fade and no overlap cover still get the short
/// [`CLIP_DECLICK_FRAMES`] ramp, so a trimmed, split or butt-joined clip
/// cannot step the signal on its first and last frame.
///
/// Shared verbatim by the live mixer and the offline bounce/export (both
/// reach it through `render_block`), so playback and bounced WAV render
/// identically. Allocation-free and `O(1)` per output frame (the
/// per-clip crossfade scan is `O(clips)`, run once per clip per block).
#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
pub fn mix_track_clips<C: Borrow<AudioClip>>(
    clips: &[C],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
) -> bool {
    mix_track_clips_governed(
        clips,
        track_id,
        playhead,
        frames,
        track_buf_l,
        track_buf_r,
        &CompRenderTable::default(),
    )
}

/// As [`mix_track_clips`], but skips every clip the take-comp table marks
/// as governed — a recorded take under comp control, which
/// [`mix_track_comp`](crate::mixer::mix_track_comp) renders instead. Without
/// this the raw, fully overlapping loop passes would all play at once, on
/// top of the comp.
///
/// Governed clips are also invisible to the automatic same-track crossfade
/// of the surviving clips: three stacked takes are not three overlapping
/// clips to be crossfaded, they are alternates of one part.
///
/// An empty table is the no-take-groups case and short-circuits every
/// governance check, so a project without take lanes renders exactly as it
/// did before.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mix_track_clips_governed<C: Borrow<AudioClip>>(
    clips: &[C],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    take_comp: &CompRenderTable,
) -> bool {
    let buf_start = playhead;
    let buf_end = playhead + frames as u64;
    let mut has_audio = false;
    let governed = !take_comp.is_empty();

    for clip in clips.iter() {
        let clip: &AudioClip = clip.borrow();
        if clip.track_id != track_id {
            continue;
        }
        if governed && take_comp.is_governed(clip.id) {
            continue;
        }

        let clip_frames = clip.duration_frames();
        let clip_start = clip.start_sample;
        let clip_end = clip_start + clip_frames;

        if buf_end <= clip_start || buf_start >= clip_end {
            continue;
        }

        let overlap_start = buf_start.max(clip_start);
        let overlap_end = buf_end.min(clip_end);

        // Fold the automatic same-track crossfade into the fade lengths:
        // an overlap at the clip's head/tail behaves like a fade of that
        // length, and the explicit fade wins only when it is longer.
        let (head_xfade, tail_xfade) =
            clip_crossfade_lengths(clip, clips, clip_frames, take_comp);
        // Anti-click ramp on both audible edges — see `CLIP_DECLICK_FRAMES`.
        // Whichever of the three is longest shapes the edge, so an explicit
        // fade or a crossfade always subsumes the declick.
        let declick = declick_frames(clip_frames);
        let fade_in_len = clip.fade_in_frames.max(head_xfade).max(declick);
        let fade_out_len = clip.fade_out_frames.max(tail_xfade).max(declick);
        let gain_lin = if clip.gain_db == 0.0 {
            1.0
        } else {
            10f32.powf(clip.gain_db / 20.0)
        };

        // Read the retuned cache when the clip carries vocal-tuning edits,
        // else the original PCM. Both share the same frame layout, so the
        // trim/fade/gain indexing below is identical; untuned clips hit the
        // zero-overhead source path. Shared by live playback and the
        // offline bounce/export, so corrected audio is identical (todo #358).
        let clip_data = clip.render_frames();
        for timeline_frame in overlap_start..overlap_end {
            let frame_offset = (timeline_frame - buf_start) as usize;
            let clip_frame =
                (timeline_frame - clip_start) as usize + clip.trim_start_frames as usize;
            let clip_idx = clip_frame * 2;
            if clip_idx + 1 < clip_data.len() {
                let coef = clip_fade_gain_coef(
                    timeline_frame,
                    clip_start,
                    clip_end,
                    fade_in_len,
                    fade_out_len,
                    clip.fade_in_curve,
                    clip.fade_out_curve,
                    gain_lin,
                );
                track_buf_l[frame_offset] += clip_data[clip_idx] * coef;
                track_buf_r[frame_offset] += clip_data[clip_idx + 1] * coef;
                has_audio = true;
            }
        }
    }

    has_audio
}

/// Length of the automatic anti-click ("declick") ramp applied to both
/// audible edges of every audio clip: 2 ms at 48 kHz.
///
/// A clip edge is a splice. Trimming a take, splitting it, or butting two
/// takes together almost never lands on a zero crossing, so playing the
/// raw samples steps the signal from silence to whatever the waveform
/// happened to be doing — an audible click, and one that a downstream amp
/// sim or delay then amplifies and repeats. Every clip therefore ramps in
/// and out over this many frames unless a longer explicit fade or an
/// overlap crossfade already shapes that edge.
///
/// 2 ms is long enough to remove the step for the lowest musical
/// fundamentals and short enough to leave a transient sliced at its attack
/// sounding like a transient (Ardour declicks over ~64 frames, Reaper over
/// 10 ms; this sits deliberately between them). Expressed in frames rather
/// than seconds because the clip mix is rate-agnostic — at 44.1 kHz it is
/// 2.2 ms, which is the same thing musically.
pub const CLIP_DECLICK_FRAMES: u64 = 96;

/// The declick ramp length for a clip of `clip_frames` audible frames,
/// capped at half the clip so the head and tail ramps of a very short clip
/// (a sliced grain, a drum hit) meet at its midpoint instead of overlapping
/// into a double attenuation.
#[inline]
fn declick_frames(clip_frames: u64) -> u64 {
    CLIP_DECLICK_FRAMES.min(clip_frames / 2)
}

/// Linear gain coefficient applied to `clip` at absolute timeline frame
/// `timeline_frame`, combining the fade-in ramp, the fade-out ramp, and
/// the clip's (already linearised) gain. `clip_end` is exclusive.
#[inline]
#[allow(clippy::too_many_arguments)]
fn clip_fade_gain_coef(
    timeline_frame: u64,
    clip_start: u64,
    clip_end: u64,
    fade_in_len: u64,
    fade_out_len: u64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
    gain_lin: f32,
) -> f32 {
    let mut coef = gain_lin;
    if fade_in_len > 0 {
        let pos = timeline_frame - clip_start;
        if pos < fade_in_len {
            coef *= fade_in_curve.coefficient(pos as f32 / fade_in_len as f32);
        }
    }
    if fade_out_len > 0 {
        // Frames remaining before the clip's last visible frame; the
        // curve runs the complementary direction (`coefficient(0)` at the
        // final frame), which equal-power turns into the constant-power
        // crossfade complement.
        let pos_from_end = (clip_end - 1).saturating_sub(timeline_frame);
        if pos_from_end < fade_out_len {
            coef *= fade_out_curve.coefficient(pos_from_end as f32 / fade_out_len as f32);
        }
    }
    coef
}

/// Lengths (in frames) of the automatic crossfades at `clip`'s head and
/// tail, derived from where other clips on the same track overlap it. The
/// head length is the span an earlier-starting clip covers from `clip`'s
/// start; the tail length is the span a later-starting clip covers up to
/// `clip`'s end. Each is capped at the clip's visible duration so a clip
/// overlapped on both sides cannot fade past its own length.
fn clip_crossfade_lengths<C: Borrow<AudioClip>>(
    clip: &AudioClip,
    clips: &[C],
    clip_frames: u64,
    take_comp: &CompRenderTable,
) -> (u64, u64) {
    let clip_start = clip.start_sample;
    let clip_end = clip_start + clip_frames;
    let governed = !take_comp.is_empty();
    let mut head = 0u64;
    let mut tail = 0u64;
    for other in clips.iter() {
        let other: &AudioClip = other.borrow();
        if other.id == clip.id || other.track_id != clip.track_id {
            continue;
        }
        // A take clip under comp control never crossfades against the
        // surviving clips — the comp pass renders it instead.
        if governed && take_comp.is_governed(other.id) {
            continue;
        }
        let o_start = other.start_sample;
        let o_end = o_start + other.duration_frames();
        // An earlier-or-equal-starting clip covering this clip's start →
        // crossfade in over the covered span.
        if o_start <= clip_start && o_end > clip_start {
            head = head.max(o_end.min(clip_end) - clip_start);
        }
        // A later-starting clip overlapping this clip's tail → crossfade
        // out over the span from where it starts to this clip's end.
        if o_start > clip_start && o_start < clip_end {
            tail = tail.max(clip_end - o_start);
        }
    }
    (head.min(clip_frames), tail.min(clip_frames))
}
