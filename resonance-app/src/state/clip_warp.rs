//! Clip warp ("follow tempo") — the app-side mirror of an audio clip's
//! warp settings and the transient UI state its editors carry.
//!
//! The engine owns the authoritative copy on its `AudioClip`
//! (`warp_enabled`, `original_bpm`, `transpose_semitones`,
//! `warp_algorithm`, `warp_markers`), set by `AudioCommand::SetClipWarp`
//! / `SetClipWarpMarkers` and echoed back as `ClipWarpChanged` /
//! `ClipWarpMarkersChanged`. [`ClipWarpState`] mirrors it on
//! [`ClipState::warp`](super::ClipState), is persisted in the project
//! file (`ProjectClip::warp`) and therefore rides every undo snapshot.
//!
//! **Marker geometry.** A [`WarpMarker`] pins `source_frame` (a frame of
//! the clip's source audio, independent of trim) to `timeline_beat`
//! (beats from the clip's start). Beats are projected onto the timeline
//! at the project's base tempo — the same `beats × 60 / bpm` projection
//! the engine's `AudioClip::warp_source_frame` uses — not through the
//! tempo map.

use resonance_audio::types::{ClipId, WarpAlgorithm, WarpMarker};

/// Lowest source tempo the app accepts, in BPM. Anything slower is not a
/// tempo a recording was performed at, and a near-zero tempo would make
/// the stretch ratio explode.
pub const MIN_WARP_BPM: f32 = 20.0;
/// Highest source tempo the app accepts, in BPM.
pub const MAX_WARP_BPM: f32 = 999.0;
/// Widest transpose the stretcher supports (±4 octaves, the
/// `resonance-dsp` `TimeStretch` clamp).
pub const MAX_TRANSPOSE_SEMITONES: f32 = 48.0;
/// Most warp markers one clip may carry. Far more than any real edit
/// needs; it bounds what a control client can make the app draw.
pub const MAX_WARP_MARKERS: usize = 1024;
/// Smallest gap, in beats, a dragged marker keeps from its neighbours
/// (a 1/64 note), so the sorted order the engine relies on never
/// collapses into two markers at one beat.
pub const MIN_WARP_MARKER_GAP_BEATS: f64 = 1.0 / 16.0;

/// GUI-side mirror of one audio clip's warp settings. `Default` is an
/// unwarped clip: warp off, unknown source tempo, no transpose, the
/// engine's default algorithm and no markers — exactly what a fresh
/// engine clip holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClipWarpState {
    /// Whether warp is switched on for the clip.
    pub enabled: bool,
    /// The tempo the source was performed at, if known.
    pub original_bpm: Option<f32>,
    /// Pitch shift applied on the warp path, in semitones.
    pub transpose_semitones: f32,
    /// Resynthesis algorithm used while warping.
    pub algorithm: WarpAlgorithm,
    /// Warp markers, sorted by `timeline_beat` ascending (the engine's
    /// invariant; the mirror upholds it too).
    pub markers: Vec<WarpMarker>,
}

impl ClipWarpState {
    /// True when nothing departs from an unwarped clip — the state the
    /// project file omits and a load sends nothing for.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The four scalar parameters `SetClipWarp` carries differ.
    pub fn params_differ(&self, other: &Self) -> bool {
        self.enabled != other.enabled
            || self.original_bpm != other.original_bpm
            || self.transpose_semitones != other.transpose_semitones
            || self.algorithm != other.algorithm
    }

    /// Source frames that pass per timeline beat when no marker governs
    /// the read: the source tempo's beat length when it is known (that is
    /// what makes the clip follow the project tempo), the project's own
    /// otherwise (no stretch).
    pub fn source_frames_per_beat(&self, project_bpm: f32, sample_rate: u32) -> f64 {
        let bpm = match self.original_bpm {
            Some(bpm) if bpm > 0.0 => bpm,
            _ => project_bpm,
        };
        if bpm <= 0.0 {
            return 0.0;
        }
        sample_rate as f64 * 60.0 / bpm as f64
    }

    /// The source frame that is heard `beat` beats after the clip's start
    /// under the current warp map — what a marker added there pins, so
    /// adding a marker never changes what plays.
    ///
    /// Piecewise linear between markers; outside them the nearest
    /// segment's rate is extended, and a lone marker (or none) reads at
    /// [`source_frames_per_beat`](Self::source_frames_per_beat). With no
    /// markers the read starts at the clip's visible start,
    /// `trim_start_frames` into the source.
    pub fn source_frame_at_beat(
        &self,
        beat: f64,
        trim_start_frames: u64,
        project_bpm: f32,
        sample_rate: u32,
    ) -> u64 {
        let spb = self.source_frames_per_beat(project_bpm, sample_rate);
        let markers = &self.markers;
        let frame = match markers.len() {
            0 => trim_start_frames as f64 + beat * spb,
            1 => markers[0].source_frame as f64 + (beat - markers[0].timeline_beat) * spb,
            n => {
                // The segment that brackets `beat`, or the first / last one
                // to extrapolate from.
                let i = markers
                    .windows(2)
                    .position(|w| beat <= w[1].timeline_beat)
                    .unwrap_or(n - 2);
                let (a, b) = (&markers[i], &markers[i + 1]);
                let span = b.timeline_beat - a.timeline_beat;
                let slope = if span > 0.0 {
                    (b.source_frame as f64 - a.source_frame as f64) / span
                } else {
                    spb
                };
                a.source_frame as f64 + (beat - a.timeline_beat) * slope
            }
        };
        frame.max(0.0).round() as u64
    }

    /// The marker set with one marker added at `beat` beats into the clip,
    /// pinning the source frame the current warp map plays there — so
    /// adding a marker changes nothing audible until it is dragged. `None`
    /// when `beat` is not a finite position `>= 0`, a marker already sits
    /// within [`MIN_WARP_MARKER_GAP_BEATS`] of it, or the clip is at
    /// [`MAX_WARP_MARKERS`].
    pub fn markers_with_added(
        &self,
        beat: f64,
        trim_start_frames: u64,
        project_bpm: f32,
        sample_rate: u32,
    ) -> Option<Vec<WarpMarker>> {
        if !beat.is_finite()
            || beat < 0.0
            || self.markers.len() >= MAX_WARP_MARKERS
            || self
                .markers
                .iter()
                .any(|m| (m.timeline_beat - beat).abs() < MIN_WARP_MARKER_GAP_BEATS)
        {
            return None;
        }
        let source_frame =
            self.source_frame_at_beat(beat, trim_start_frames, project_bpm, sample_rate);
        let mut markers = self.markers.clone();
        markers.push(WarpMarker {
            source_frame,
            timeline_beat: beat,
        });
        sort_warp_markers(&mut markers);
        Some(markers)
    }

    /// The marker set without marker `index` (unchanged when out of range).
    pub fn markers_without(&self, index: usize) -> Vec<WarpMarker> {
        let mut markers = self.markers.clone();
        if index < markers.len() {
            markers.remove(index);
        }
        markers
    }
}

/// A tempo for display: two decimals, trailing zeros dropped (`120`,
/// `123.5`, `99.25`).
pub fn format_warp_bpm(bpm: f32) -> String {
    let s = format!("{bpm:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Sort markers into the engine's order (ascending `timeline_beat`).
pub fn sort_warp_markers(markers: &mut [WarpMarker]) {
    markers.sort_by(|a, b| a.timeline_beat.total_cmp(&b.timeline_beat));
}

/// A source tempo the app accepts: finite and positive, clamped into
/// [`MIN_WARP_BPM`]`..=`[`MAX_WARP_BPM`]. `None` for anything else.
pub fn clamp_warp_bpm(bpm: f32) -> Option<f32> {
    (bpm.is_finite() && bpm > 0.0).then(|| bpm.clamp(MIN_WARP_BPM, MAX_WARP_BPM))
}

/// Clamp a transpose into ±[`MAX_TRANSPOSE_SEMITONES`]; `NaN` is no shift.
pub fn clamp_transpose(semitones: f32) -> f32 {
    if semitones.is_nan() {
        0.0
    } else {
        semitones.clamp(-MAX_TRANSPOSE_SEMITONES, MAX_TRANSPOSE_SEMITONES)
    }
}

/// Where a clip's tempo detection stands. Transient UI state — the
/// detection changes nothing until the user applies its result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TempoDetectStatus {
    /// `DetectClipTempo` was sent; no reply yet.
    Running,
    /// The detector found a tempo.
    Detected { bpm: f32, confidence: f32 },
    /// The detector ran and found no tempo (too short, or no pulse).
    NotFound,
}

/// An in-progress drag of one warp marker. The marker keeps its
/// `source_frame` and slides along the timeline; `index` stays valid for
/// the whole gesture because the drag is clamped between the marker's
/// neighbours, so the set never reorders.
#[derive(Debug, Clone, PartialEq)]
pub struct WarpMarkerDragState {
    pub clip_id: ClipId,
    pub index: usize,
}

/// The inspector's source-tempo field while the user is typing in it.
/// Committed on Enter; a draft for another clip is ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct WarpBpmDraft {
    pub clip_id: ClipId,
    pub text: String,
}
