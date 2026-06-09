//! The [`Assistant`] facade: owns the capture ring and the latest
//! analysis/suggestion/reference state behind mutexes, and exposes the
//! audio-thread feed plus the UI-thread analyze/apply entry points.

use parking_lot::Mutex;

use super::analyze::{self, AnalysisResult};
use super::capture::CaptureBuffer;
use super::decide::{self, Suggestions, Target};
use super::reference::{self, ReferenceTrack};

/// Capture duration in seconds. The research brief explicitly calls
/// for "play ~10 seconds, run analysis".
pub const CAPTURE_SECONDS: f32 = 10.0;

pub struct Assistant {
    capture: Mutex<CaptureBuffer>,
    last_analysis: Mutex<Option<AnalysisResult>>,
    last_suggestions: Mutex<Option<Suggestions>>,
    reference: Mutex<Option<ReferenceTrack>>,
    reference_error: Mutex<Option<String>>,
}

impl Assistant {
    pub fn new(sample_rate: f32) -> Self {
        let capacity = (CAPTURE_SECONDS * sample_rate) as usize;
        Self {
            capture: Mutex::new(CaptureBuffer::new(capacity, sample_rate)),
            last_analysis: Mutex::new(None),
            last_suggestions: Mutex::new(None),
            reference: Mutex::new(None),
            reference_error: Mutex::new(None),
        }
    }

    pub fn set_sample_rate(&self, sample_rate: f32) {
        let expected = (CAPTURE_SECONDS * sample_rate) as usize;
        let mut cap = self.capture.lock();
        if cap.capacity() != expected {
            *cap = CaptureBuffer::new(expected, sample_rate);
        } else {
            cap.set_sample_rate(sample_rate);
        }
    }

    /// Audio-thread hot path: append a stereo block to the capture ring.
    ///
    /// Uses `try_lock` to guarantee the audio thread never blocks waiting
    /// on the UI's `snapshot_chrono` (which clones two ~480 k-sample
    /// vecs and can take milliseconds — longer than one audio block).
    /// When the lock is held by the UI, the block is dropped; the ring
    /// loses at most a few hundred milliseconds across a snapshot call,
    /// which is acceptable for a 10-second offline-analysis window.
    pub fn feed(&self, left: &[f32], right: &[f32]) {
        if let Some(mut cap) = self.capture.try_lock() {
            cap.push(left, right);
        }
    }

    /// How much of the capture ring currently holds real audio,
    /// as a fraction from 0.0 (empty) to 1.0 (full).
    pub fn capture_fraction(&self) -> f32 {
        let c = self.capture.lock();
        c.filled() as f32 / c.capacity().max(1) as f32
    }

    /// Run analysis on the current ring contents against the given
    /// target. Returns `None` if the ring holds less than 2 seconds
    /// of audio (not enough for a stable LUFS-I gated measurement).
    pub fn analyze(&self, target: Target) -> Option<Suggestions> {
        let (l, r, sr) = {
            let cap = self.capture.lock();
            let min_samples = (cap.sample_rate() * 2.0) as usize;
            if cap.filled() < min_samples {
                return None;
            }
            let sr = cap.sample_rate();
            let (l, r) = cap.snapshot_chrono();
            (l, r, sr)
        };

        let analysis = analyze::run(sr, &l, &r);
        let suggestions = decide::build(&analysis, &target);
        *self.last_analysis.lock() = Some(analysis);
        *self.last_suggestions.lock() = Some(suggestions.clone());
        Some(suggestions)
    }

    /// Load a reference track from disk. On success, stores the
    /// decoded track so the next `analyze` call can target it. On
    /// failure, stores the error for the UI to display.
    pub fn load_reference(&self, path: &str) -> Result<(), String> {
        match reference::load_from_path(path) {
            Ok(track) => {
                *self.reference.lock() = Some(track);
                *self.reference_error.lock() = None;
                Ok(())
            }
            Err(e) => {
                *self.reference.lock() = None;
                *self.reference_error.lock() = Some(e.clone());
                Err(e)
            }
        }
    }

    pub fn reference(&self) -> Option<ReferenceTrack> {
        self.reference.lock().clone()
    }

    pub fn reference_error(&self) -> Option<String> {
        self.reference_error.lock().clone()
    }

    pub fn clear_reference(&self) {
        *self.reference.lock() = None;
        *self.reference_error.lock() = None;
    }

    pub fn last_analysis(&self) -> Option<AnalysisResult> {
        self.last_analysis.lock().clone()
    }

    pub fn last_suggestions(&self) -> Option<Suggestions> {
        self.last_suggestions.lock().clone()
    }

    pub fn clear(&self) {
        self.capture.lock().clear();
        *self.last_analysis.lock() = None;
        *self.last_suggestions.lock() = None;
    }
}

impl Assistant {
    /// Install a pre-constructed reference track directly, without
    /// hitting the filesystem. Used by integration tests that want to
    /// exercise the reference-based analysis path with a synthetic
    /// track.
    pub fn set_reference_for_testing(&self, track: ReferenceTrack) {
        *self.reference.lock() = Some(track);
    }
}
