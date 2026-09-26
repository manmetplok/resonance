//! Audio and MIDI clip data structures, plus the waveform peak helper.
//!
//! Pure data and pure helpers: the WAV file I/O these clips are loaded
//! from lives in [`crate::io::wav`], so this module stays usable without
//! a filesystem (ba todo #1260).
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{ClipId, SamplePos, TrackId};

/// A single MIDI note in a clip.
#[derive(Debug, Clone)]
pub struct MidiNote {
    pub note: u8,
    pub velocity: f32,
    pub start_tick: u64,
    pub duration_ticks: u64,
}

/// Move `notes[index]` to `(new_start_tick, new_note)` and restore the
/// start-tick order with a *stable* sort, returning the index the moved
/// note now sits at.
///
/// This is the one definition of a note move: the engine's
/// `MoveMidiNote` handler, the app's echo mirror and every drag in the
/// note editors go through it, so they agree on where the note went. A
/// caller that keeps addressing the note (the next drag step, a follow-up
/// resize) must use the returned index — the old one may now name a
/// neighbour. Out-of-range `index` is a no-op that returns `index`.
pub fn move_note_resorted(
    notes: &mut [MidiNote],
    index: usize,
    new_start_tick: u64,
    new_note: u8,
) -> usize {
    if index >= notes.len() {
        return index;
    }
    // A stable sort puts the moved note after every note that starts
    // earlier, and after the equal-start notes that preceded it.
    let new_index = notes
        .iter()
        .enumerate()
        .filter(|&(i, n)| {
            i != index
                && (n.start_tick < new_start_tick || (n.start_tick == new_start_tick && i < index))
        })
        .count();
    notes[index].start_tick = new_start_tick;
    notes[index].note = new_note;
    notes.sort_by_key(|n| n.start_tick);
    new_index
}

/// A MIDI clip containing note data, placed on the timeline.
#[derive(Debug)]
pub struct MidiClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Position on the timeline in samples (same units as AudioClip).
    pub start_sample: SamplePos,
    /// Logical length in ticks.
    pub duration_ticks: u64,
    /// Notes sorted by start_tick.
    pub notes: Vec<MidiNote>,
    pub name: String,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
}

impl MidiClip {
    /// Visible duration in ticks after trim.
    pub fn visible_duration_ticks(&self) -> u64 {
        self.duration_ticks
            .saturating_sub(self.trim_start_ticks)
            .saturating_sub(self.trim_end_ticks)
    }

    /// Convert visible duration to samples using the tempo map.
    pub fn duration_samples(&self, samples_per_tick: f64) -> u64 {
        (self.visible_duration_ticks() as f64 * samples_per_tick) as u64
    }

    /// End position on timeline in samples.
    pub fn end_sample(&self, samples_per_tick: f64) -> SamplePos {
        self.start_sample + self.duration_samples(samples_per_tick)
    }
}

/// A note event to be sent to a plugin during audio processing.
#[derive(Debug, Clone)]
pub struct PendingNoteEvent {
    pub is_note_on: bool,
    pub note: u8,
    pub velocity: f32,
    pub sample_offset: u32,
}

/// Backing storage for a clip's PCM samples. Recorded clips and clips
/// loaded from a project on disk are USUALLY `Mapped` (memory-mapped
/// WAV files); clips that were just decoded in memory but not yet
/// persisted use `Memory`, as does a WAV whose data chunk is not
/// 4-byte aligned (see [`ClipSource::open_wav`]). Do not rely on
/// "project audio is always Mapped" — it holds for every file this
/// app writes, but not for one authored elsewhere.
#[derive(Debug)]
pub enum ClipSource {
    /// Owned, in-RAM stereo-interleaved f32 samples.
    Memory(Vec<f32>),
    /// Memory-mapped WAV file sliced down to the PCM data chunk.
    /// `data_offset_bytes` is the byte offset from the start of the
    /// mapping where interleaved f32 samples begin; `frame_count` is
    /// the number of stereo frames stored in the data chunk. `path`
    /// is the on-disk location of the file backing the mapping,
    /// retained so save-to-a-new-directory can copy it.
    Mapped {
        mmap: Arc<memmap2::Mmap>,
        data_offset_bytes: usize,
        frame_count: u64,
        path: PathBuf,
    },
}

impl ClipSource {
    /// A second handle on the same audio, for a clip split in two.
    ///
    /// `Mapped` shares its mmap through the `Arc` — free, and the case
    /// that matters (recorded takes and project audio are mapped).
    /// `Memory` has to copy: an owned `Vec` cannot be shared, and a
    /// split is a deliberate, one-off edit rather than a hot path.
    pub fn share(&self) -> ClipSource {
        match self {
            ClipSource::Memory(v) => ClipSource::Memory(v.clone()),
            ClipSource::Mapped {
                mmap,
                data_offset_bytes,
                frame_count,
                path,
            } => ClipSource::Mapped {
                mmap: Arc::clone(mmap),
                data_offset_bytes: *data_offset_bytes,
                frame_count: *frame_count,
                path: path.clone(),
            },
        }
    }

    /// Stereo-interleaved f32 samples as a slice: one `[l, r]` pair
    /// per frame. This is called from the mixer hot path, so it must
    /// be O(1) and allocation-free.
    #[inline]
    pub fn as_frames(&self) -> &[f32] {
        match self {
            ClipSource::Memory(v) => v.as_slice(),
            ClipSource::Mapped {
                mmap,
                data_offset_bytes,
                frame_count,
                ..
            } => {
                let byte_len = (*frame_count as usize) * 2 * std::mem::size_of::<f32>();
                let bytes = &mmap[*data_offset_bytes..*data_offset_bytes + byte_len];
                bytemuck::cast_slice::<u8, f32>(bytes)
            }
        }
    }

    /// Total number of stereo frames in the underlying PCM data.
    #[inline]
    pub fn frame_count(&self) -> u64 {
        match self {
            ClipSource::Memory(v) => (v.len() / 2) as u64,
            ClipSource::Mapped { frame_count, .. } => *frame_count,
        }
    }

    /// Open a 32-bit-float stereo WAV file, memory-map it, and return a
    /// `Mapped` ClipSource referencing its PCM data chunk. Also
    /// pre-touches every page to avoid major page faults on the audio
    /// thread the first time the clip is played.
    ///
    /// Returns `Memory` instead when the data chunk is not 4-byte
    /// aligned — see [`ClipSource::open_wav_inner`]. Every WAV this app
    /// writes is aligned, so that is a fallback for externally-authored
    /// files, not a path the app takes on its own output.
    pub fn open_wav(path: &Path) -> Result<Self, String> {
        Self::open_wav_inner(path).map(|(source, _)| source)
    }

    /// Like [`ClipSource::open_wav`], but compares the WAV's fmt-chunk
    /// sample rate against `engine_sample_rate`. On mismatch (e.g. a
    /// project recorded under one PipeWire rate opened while the engine
    /// runs at another) the PCM data is resampled to the engine rate
    /// and returned as an in-RAM `Memory` source, so the clip plays at
    /// the correct pitch and speed. The resample happens at load time,
    /// off the audio thread; the next project save re-encodes the clip
    /// to disk at the engine rate.
    pub fn open_wav_at_rate(path: &Path, engine_sample_rate: u32) -> Result<Self, String> {
        let (source, wav_sample_rate) = Self::open_wav_inner(path)?;
        if wav_sample_rate == engine_sample_rate {
            return Ok(source);
        }
        Ok(ClipSource::Memory(crate::decode::linear_resample(
            source.as_frames(),
            wav_sample_rate,
            engine_sample_rate,
        )))
    }

    /// Map the file and wrap it as a `Mapped` source, returning the
    /// WAV's own sample rate alongside. The file I/O, the mapping and
    /// the RIFF parse all live in [`crate::io::wav`]; this only puts the
    /// result into the clip's data shape.
    /// A data chunk that is not 4-byte aligned falls back to an in-RAM
    /// `Memory` copy: RIFF only requires chunks to be *word* (2-byte)
    /// aligned, so an odd-length chunk ahead of `data` — a `LIST`/`INFO`
    /// block with an odd-length string, which several DAWs write — can
    /// leave the samples on a 2-byte boundary. `as_frames` casts the
    /// mapped bytes to `&[f32]` with `bytemuck::cast_slice`, which
    /// PANICS on misalignment, and it runs on the audio thread. Copying
    /// at load time costs one allocation off the RT path and keeps the
    /// hot accessor a plain slice cast.
    fn open_wav_inner(path: &Path) -> Result<(Self, u32), String> {
        let mapped = crate::io::wav::map_wav_file(path).map_err(|e| e.to_string())?;
        let aligned = (mapped.mmap.as_ptr() as usize + mapped.data_offset_bytes)
            % std::mem::align_of::<f32>()
            == 0;
        if !aligned {
            let byte_len = (mapped.frame_count as usize) * 2 * std::mem::size_of::<f32>();
            let bytes = &mapped.mmap[mapped.data_offset_bytes..mapped.data_offset_bytes + byte_len];
            // Decode per 4-byte group rather than casting the slice:
            // `bytemuck::cast_slice::<u8, f32>` has the SAME alignment
            // precondition we are here to avoid, so using it would
            // panic on exactly the input this branch exists for.
            // `from_le_bytes` has no alignment requirement, and this
            // also skips the zero-fill a `vec![0.0; n]` + copy would do.
            let samples: Vec<f32> = bytes
                .chunks_exact(std::mem::size_of::<f32>())
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            return Ok((ClipSource::Memory(samples), mapped.sample_rate));
        }
        Ok((
            ClipSource::Mapped {
                mmap: Arc::new(mapped.mmap),
                data_offset_bytes: mapped.data_offset_bytes,
                frame_count: mapped.frame_count,
                path: path.to_path_buf(),
            },
            mapped.sample_rate,
        ))
    }

    /// On-disk path backing a `Mapped` source, or `None` for `Memory`.
    ///
    /// Test-only in practice: the save path destructures
    /// `ClipSource::Mapped { path, .. }` directly rather than calling
    /// this, so "fixing save-as" by editing this accessor would change
    /// nothing.
    pub fn mapped_path(&self) -> Option<&Path> {
        match self {
            ClipSource::Mapped { path, .. } => Some(path.as_path()),
            ClipSource::Memory(_) => None,
        }
    }
}

/// Shape of a fade ramp (and, where two clips overlap, the automatic
/// crossfade derived from their adjacent fades). Each variant maps a
/// normalized fade-in position `t` in `[0, 1]` to a linear gain
/// coefficient via [`FadeCurve::coefficient`].
///
/// - `Linear`: `t` — constant-slope amplitude ramp.
/// - `EqualPower`: `sin(t·π/2)` — constant-power ramp; two clips whose
///   adjacent fades are equal-power sum to constant power across an
///   overlap, giving a click-free crossfade seam. This is the default.
/// - `Exp`: `t²` — slow-start exponential-ish ramp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FadeCurve {
    Linear,
    EqualPower,
    Exp,
}

impl Default for FadeCurve {
    fn default() -> Self {
        FadeCurve::EqualPower
    }
}

impl FadeCurve {
    /// Linear gain coefficient for this curve at normalized fade-in
    /// position `t`. `t` is the progress through a fade-in, in `[0, 1]`:
    /// `0.0` → silence, `1.0` → unity. For a fade-out, pass the
    /// complementary position `1.0 - t` so the ramp runs the other way
    /// (`EqualPower` is symmetric: `sin((1−t)·π/2) = cos(t·π/2)`, the
    /// constant-power complement the crossfade math relies on).
    ///
    /// `t` outside `[0, 1]` is clamped, so callers can hand in raw
    /// `frame / fade_frames` ratios without a separate bounds check.
    /// O(1) and allocation-free for the mixer hot path.
    #[inline]
    pub fn coefficient(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => t,
            FadeCurve::EqualPower => (t * std::f32::consts::FRAC_PI_2).sin(),
            FadeCurve::Exp => t * t,
        }
    }
}

/// Resynthesis algorithm used when a clip is time-stretched ("warped").
/// This is data only — the stretch itself is performed on the render /
/// bounce path; the original [`ClipSource`] PCM is never mutated.
///
/// - `Tonal`: phase-vocoder-style stretch that preserves the pitch of
///   sustained, harmonic material (vocals, pads, leads) at the cost of
///   smearing sharp attacks.
/// - `Transient`: transient-preserving stretch (the default) that keeps
///   drum hits and plucked attacks crisp — the safe choice for rhythmic
///   material and unknown content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WarpAlgorithm {
    Tonal,
    #[default]
    Transient,
}

/// A warp marker pinning a point in a clip's source audio to a musical
/// position on the timeline. Between adjacent markers the source is read
/// at a locally uniform rate so that `source_frame` is heard exactly at
/// `timeline_beat`; this is what lets a clip be elastically aligned to a
/// tempo map or hand-corrected for groove. See
/// [`AudioClip::warp_source_frame`] for the mapping these drive.
///
/// Markers belonging to a clip are kept sorted by `timeline_beat`
/// ascending (with `source_frame` likewise non-decreasing for a sane,
/// non-reversing warp).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WarpMarker {
    /// Position in the clip's source PCM, in stereo sample frames measured
    /// from the start of the audio data — independent of trim and of where
    /// the clip sits on the timeline.
    pub source_frame: u64,
    /// Musical position, in beats relative to the clip's start, at which
    /// `source_frame` should be heard.
    pub timeline_beat: f64,
}

/// An audio clip on the timeline. The PCM samples live behind
/// [`ClipSource`], which may be an owned `Vec<f32>` or a
/// memory-mapped WAV file — so large recorded takes never need to
/// inflate into a contiguous in-RAM buffer.
#[derive(Debug)]
pub struct AudioClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Start position on the timeline in samples.
    pub start_sample: SamplePos,
    pub source: ClipSource,
    /// Original file name.
    pub name: String,
    /// Non-destructive trim: frames to skip from the start of audio data.
    pub trim_start_frames: u64,
    /// Non-destructive trim: frames to skip from the end of audio data.
    pub trim_end_frames: u64,
    /// Fade-in length in frames, ramping up over the first
    /// `fade_in_frames` after the clip's visible start. `0` = no fade.
    pub fade_in_frames: u64,
    /// Curve shaping the fade-in ramp.
    pub fade_in_curve: FadeCurve,
    /// Fade-out length in frames, ramping down over the last
    /// `fade_out_frames` before the clip's visible end. `0` = no fade.
    pub fade_out_frames: u64,
    /// Curve shaping the fade-out ramp.
    pub fade_out_curve: FadeCurve,
    /// Per-clip gain in decibels. `0.0` dB = unity (no change).
    pub gain_db: f32,
    /// Non-destructive vocal pitch/timing correction. `None` (the default)
    /// means the clip is untuned and incurs zero overhead — existing
    /// behaviour. `Some(_)` holds the analysis cache plus per-note and
    /// global edits; the original [`ClipSource`] PCM is never mutated.
    pub vocal_tuning: Option<super::VocalTuning>,
    /// Whether elastic time-stretching ("warp") is applied to this clip.
    /// `false` (the default) reads the source 1:1, so existing clips are
    /// unchanged and incur zero overhead.
    pub warp_enabled: bool,
    /// Tempo, in BPM, the source material was recorded / performed at.
    /// `None` (the default) means unknown; with no warp markers this makes
    /// warp fall back to no stretch (a unit read ratio).
    pub original_bpm: Option<f32>,
    /// Global pitch shift applied on the warp / resynthesis path, in
    /// semitones. `0.0` (the default) = no transpose. Stored independently
    /// of stretch so a clip can be pitched without being time-stretched.
    pub transpose_semitones: f32,
    /// Resynthesis algorithm used while warping.
    pub warp_algorithm: WarpAlgorithm,
    /// Warp markers pinning source frames to timeline beats, kept sorted by
    /// `timeline_beat`. Empty (the default) → a single uniform stretch
    /// governed by `original_bpm`; non-empty → piecewise-linear warp.
    pub warp_markers: Vec<WarpMarker>,
    /// Derived, **non-persisted** render cache holding the formant-preserving
    /// retuned PCM for this clip (interleaved stereo, exactly the same frame
    /// count as [`ClipSource::as_frames`]). Built off the realtime thread by
    /// [`crate::engine::vocal_render::ensure_tuning_caches`] whenever
    /// [`Self::vocal_tuning`] carries edits, and read on the hot mixer path
    /// via [`Self::render_frames`]. `None` when the clip is untuned or its
    /// tuning is the identity edit — the zero-overhead path.
    ///
    /// NOTE: the cache is only ever built by the four OFFLINE paths
    /// (bounce / wav / stem / freeze); nothing on the engine control
    /// thread calls `ensure_tuning_caches`. Live playback therefore
    /// reflects a tuning edit only after an export has run — this is NOT
    /// the "identical in playback and bounce" guarantee this doc used to
    /// claim. See the module note on
    /// [`crate::engine::vocal_render`] before depending on it.
    /// The original [`ClipSource`] PCM is never mutated.
    pub tuning_render_cache: Option<Vec<f32>>,
}

/// Number of stereo frames per waveform peak bucket.
pub const WAVEFORM_PEAK_FRAMES: usize = 512;

/// Compute downsampled waveform peaks from stereo interleaved audio data.
/// Returns (min, max) pairs, one per chunk of `WAVEFORM_PEAK_FRAMES` frames.
/// Uses the mono mix (L+R)/2 for display.
pub fn compute_waveform_peaks(data: &[f32]) -> Vec<(f32, f32)> {
    let total_frames = data.len() / 2;
    let num_peaks = total_frames.div_ceil(WAVEFORM_PEAK_FRAMES);
    let mut peaks = Vec::with_capacity(num_peaks);
    for chunk_start in (0..total_frames).step_by(WAVEFORM_PEAK_FRAMES) {
        let chunk_end = (chunk_start + WAVEFORM_PEAK_FRAMES).min(total_frames);
        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;
        for f in chunk_start..chunk_end {
            let mono = (data[f * 2] + data[f * 2 + 1]) * 0.5;
            if mono < min_val {
                min_val = mono;
            }
            if mono > max_val {
                max_val = mono;
            }
        }
        peaks.push((min_val, max_val));
    }
    peaks
}

/// True when any audio clip on `track_id` overlaps the half-open timeline
/// window `[start, end)` (sample frames). This is the "covered span" test
/// of the external-instrument Recorded playback mode (doc #257): over
/// covered spans the recorded take plays and the MIDI-out / monitor mix
/// are gated; outside them the track falls back to live. A clip's extent
/// is its visible (post-trim) `[start_sample, end_sample())`, matching
/// exactly what `mix_track_clips` will audibly play. Zero-length windows
/// and empty (fully trimmed) clips cover nothing. `O(clips)`,
/// allocation-free — safe on both the audio callback and the engine
/// control thread.
pub fn audio_clip_covers(clips: &[AudioClip], track_id: TrackId, start: u64, end: u64) -> bool {
    start < end
        && clips.iter().any(|clip| {
            clip.track_id == track_id && clip.start_sample < end && clip.end_sample() > start
        })
}

impl AudioClip {
    /// Total number of frames in the raw audio data.
    pub fn total_frames(&self) -> u64 {
        self.source.frame_count()
    }

    /// Visible/audible duration in stereo sample frames (after trim).
    pub fn duration_frames(&self) -> u64 {
        self.total_frames()
            .saturating_sub(self.trim_start_frames)
            .saturating_sub(self.trim_end_frames)
    }

    /// End position on timeline in sample frames.
    pub fn end_sample(&self) -> SamplePos {
        self.start_sample + self.duration_frames()
    }

    /// The TAIL half of a split at absolute timeline position
    /// `at_sample`, or `None` when the position is at or outside either
    /// edge (a cut that leaves one side empty is not a cut).
    ///
    /// The caller is expected to shorten `self` to the head — see
    /// [`AudioClip::split_head_trim_end`], which computes the matching
    /// `trim_end_frames` — so the two halves together play exactly what
    /// the original did.
    ///
    /// Both halves are non-destructive trims of the same audio. A mapped
    /// source (every recorded take) is shared through its `Arc`, so the
    /// split costs nothing; an in-RAM source is COPIED, because owning a
    /// `Vec` is the one thing that cannot be shared. Warp markers and
    /// vocal tuning are deliberately dropped on the tail: both are keyed
    /// to positions in the original clip's timeline and would be wrong
    /// rather than merely absent.
    pub fn split_tail(&self, new_id: ClipId, at_sample: SamplePos) -> Option<AudioClip> {
        if at_sample <= self.start_sample || at_sample >= self.end_sample() {
            return None;
        }
        let head_frames = at_sample - self.start_sample;
        Some(AudioClip {
            id: new_id,
            track_id: self.track_id,
            start_sample: at_sample,
            source: self.source.share(),
            name: self.name.clone(),
            trim_start_frames: self.trim_start_frames + head_frames,
            trim_end_frames: self.trim_end_frames,
            // Fades follow the audible edges: the head keeps the
            // fade-in, the tail the fade-out. Copying both onto both
            // would duck the middle of what was one performance.
            fade_in_frames: 0,
            fade_in_curve: self.fade_in_curve,
            fade_out_frames: self.fade_out_frames,
            fade_out_curve: self.fade_out_curve,
            gain_db: self.gain_db,
            vocal_tuning: None,
            warp_enabled: self.warp_enabled,
            original_bpm: self.original_bpm,
            transpose_semitones: self.transpose_semitones,
            warp_algorithm: self.warp_algorithm,
            warp_markers: Vec::new(),
            tuning_render_cache: None,
        })
    }

    /// The `trim_end_frames` the HEAD of a split at `at_sample` needs, so
    /// it ends exactly where [`AudioClip::split_tail`] begins.
    pub fn split_head_trim_end(&self, at_sample: SamplePos) -> u64 {
        let head_frames = at_sample.saturating_sub(self.start_sample);
        self.trim_end_frames + self.duration_frames().saturating_sub(head_frames)
    }

    /// True when the clip carries vocal-tuning data (it has been analysed
    /// and/or edited). A clip with `Some(VocalTuning::default())` counts as
    /// tuned even before analysis populates it.
    pub fn is_tuned(&self) -> bool {
        self.vocal_tuning.is_some()
    }

    /// Stereo-interleaved frames the mixer should read for this clip: the
    /// retuned [`Self::tuning_render_cache`] when present, otherwise the
    /// original [`ClipSource`] PCM. Both buffers share the same frame
    /// layout, so the caller's trim/fade/gain indexing is unchanged.
    ///
    /// Called from the mixer hot path — O(1) and allocation-free. Untuned
    /// clips (the cache is `None`) return the source slice directly, so they
    /// keep their existing zero-overhead behaviour.
    #[inline]
    pub fn render_frames(&self) -> &[f32] {
        match &self.tuning_render_cache {
            Some(cache) => cache.as_slice(),
            None => self.source.as_frames(),
        }
    }

    /// Mutable access to the clip's [`VocalTuning`], creating an empty
    /// (default) model on first use. Use this when applying an edit or
    /// storing analysis results to a clip that may not have been tuned yet.
    pub fn vocal_tuning_mut(&mut self) -> &mut super::VocalTuning {
        self.vocal_tuning.get_or_insert_with(super::VocalTuning::default)
    }

    /// Uniform source-read ratio for the marker-free case: how many source
    /// frames advance per timeline frame. `project_bpm / original_bpm`, so a
    /// project faster than the recording reads the source faster (ratio > 1).
    /// Falls back to `1.0` (no stretch) when `original_bpm` is unknown or
    /// either tempo is non-positive — keeping untouched clips bit-identical.
    fn warp_ratio(&self, project_bpm: f32) -> f64 {
        match self.original_bpm {
            Some(original_bpm) if original_bpm > 0.0 && project_bpm > 0.0 => {
                (project_bpm / original_bpm) as f64
            }
            _ => 1.0,
        }
    }

    /// Map a timeline read position to the position in the clip's source PCM
    /// the warp engine should read from.
    ///
    /// `timeline_frame` is the playhead offset, in stereo sample frames at
    /// `sample_rate`, from the clip's start; the returned value is the
    /// (fractional) source frame to read. The original [`ClipSource`] is
    /// never mutated — this only derives where to read.
    ///
    /// - **No warp markers:** a single uniform resample at ratio
    ///   `project_bpm / original_bpm`. With an unknown `original_bpm` the
    ///   ratio is `1.0`, i.e. the identity map.
    /// - **Markers present:** each marker's `timeline_beat` is projected onto
    ///   the frame axis (`beats × sample_rate × 60 / project_bpm`) and the
    ///   source position is piecewise-linearly interpolated between the
    ///   bracketing markers. Outside the marker span the rate of the nearest
    ///   segment is extended (or the uniform ratio for a single marker).
    ///
    /// Markers are assumed sorted by `timeline_beat` ascending (the invariant
    /// documented on [`WarpMarker`]).
    pub fn warp_source_frame(
        &self,
        timeline_frame: f64,
        project_bpm: f32,
        sample_rate: u32,
    ) -> f64 {
        let ratio = self.warp_ratio(project_bpm);

        let markers = &self.warp_markers;
        if markers.is_empty() {
            return timeline_frame * ratio;
        }

        // Project a marker's musical beat onto the timeline frame axis.
        let frames_per_beat = if project_bpm > 0.0 {
            sample_rate as f64 * 60.0 / project_bpm as f64
        } else {
            0.0
        };
        let tl_frame = |m: &WarpMarker| m.timeline_beat * frames_per_beat;

        // Local source-frames-per-timeline-frame slope of a segment, falling
        // back to the uniform ratio for a degenerate (zero-width) segment.
        let slope = |a: &WarpMarker, b: &WarpMarker| {
            let (ta, tb) = (tl_frame(a), tl_frame(b));
            if tb > ta {
                (b.source_frame as f64 - a.source_frame as f64) / (tb - ta)
            } else {
                ratio
            }
        };

        let first = &markers[0];
        if timeline_frame <= tl_frame(first) {
            let s = if markers.len() >= 2 {
                slope(first, &markers[1])
            } else {
                ratio
            };
            return first.source_frame as f64 + (timeline_frame - tl_frame(first)) * s;
        }

        let last = &markers[markers.len() - 1];
        if timeline_frame >= tl_frame(last) {
            let s = if markers.len() >= 2 {
                slope(&markers[markers.len() - 2], last)
            } else {
                ratio
            };
            return last.source_frame as f64 + (timeline_frame - tl_frame(last)) * s;
        }

        for pair in markers.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let (ta, tb) = (tl_frame(a), tl_frame(b));
            if timeline_frame >= ta && timeline_frame <= tb {
                if tb <= ta {
                    return a.source_frame as f64;
                }
                let frac = (timeline_frame - ta) / (tb - ta);
                return a.source_frame as f64
                    + frac * (b.source_frame as f64 - a.source_frame as f64);
            }
        }

        // Unreachable when markers are sorted (the query lies within the span
        // handled above); return the last anchor as a safe fallback.
        last.source_frame as f64
    }
}
