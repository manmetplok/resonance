//! Live spectrum analyzer used behind the EQ response curve.
//!
//! Two shared spectrum analyzers from `resonance-metering` are fed by the
//! audio thread: one taps the input block (pre-EQ) and the other taps the
//! output block (post-EQ). The audio thread only downmixes to mono and
//! pushes samples into a lock-free SPSC ring — no FFT, no lock, no
//! allocation. A background worker per tap owns the FFT plan, drains the
//! ring at its own pace, and publishes a peak-hold-with-decay 1/6-octave
//! [`SpectrumSnapshot`] through an `ArcSwap` the editor reads wait-free
//! via [`SpectrumHandle`] on its own ~60 Hz repaint tick.
//!
//! This replaced a hand-rolled 2048-point Hann FFT that ran on the audio
//! thread and published through a `parking_lot::Mutex` — the metering
//! crate's worker stack is the workspace-standard pattern (the mastering
//! plugin uses the same one) and its smoothing was tuned to match the old
//! analyzer's feel.

use std::sync::Arc;

use parking_lot::RwLock;
use resonance_metering::SpectrumAnalyzer;

pub use resonance_metering::{SpectrumHandle, SpectrumSnapshot, NUM_OCTAVE_BINS};

// ---------------------------------------------------------------------------
// Shared handles (audio → UI).
// ---------------------------------------------------------------------------

/// Shared between the plugin struct and the editor. `initialize()` installs
/// fresh worker handles each time the audio side is (re)built, and the
/// editor re-reads them every frame, so a sample-rate change swaps the taps
/// out from under an open window without either side noticing.
pub struct AnalyzerState {
    pre: RwLock<Option<SpectrumHandle>>,
    post: RwLock<Option<SpectrumHandle>>,
}

impl AnalyzerState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pre: RwLock::new(None),
            post: RwLock::new(None),
        })
    }

    /// Latest published pre-EQ snapshot, or `None` until `initialize()`
    /// has spawned the workers.
    pub fn latest_pre(&self) -> Option<Arc<SpectrumSnapshot>> {
        self.pre.read().as_ref().map(|h| h.latest())
    }

    /// Latest published post-EQ snapshot, or `None` until `initialize()`
    /// has spawned the workers.
    pub fn latest_post(&self) -> Option<Arc<SpectrumSnapshot>> {
        self.post.read().as_ref().map(|h| h.latest())
    }

    fn install(&self, pre: SpectrumHandle, post: SpectrumHandle) {
        *self.pre.write() = Some(pre);
        *self.post.write() = Some(post);
    }
}

// ---------------------------------------------------------------------------
// Audio-thread producer side.
// ---------------------------------------------------------------------------

/// Owns the two spectrum analyzers (pre and post) and, through them, the
/// two background FFT worker threads. Lives on the plugin struct; the
/// plugin's `process()` calls `feed_pre` before the DSP runs and
/// `feed_post` after.
///
/// Dropping this joins both workers ([`SpectrumAnalyzer`]'s `Drop` signals
/// the thread and joins it), so re-running `initialize()` or dropping the
/// plugin cannot leak a thread.
pub struct StereoAnalyzers {
    pre: SpectrumAnalyzer,
    post: SpectrumAnalyzer,
}

impl StereoAnalyzers {
    /// Spawn the two background FFT workers and install their read handles
    /// into `shared` for the editor.
    pub fn new(sample_rate: f32, shared: &AnalyzerState) -> Self {
        let pre = SpectrumAnalyzer::spawn(sample_rate);
        let post = SpectrumAnalyzer::spawn(sample_rate);
        shared.install(pre.handle(), post.handle());
        Self { pre, post }
    }

    pub fn reset(&self) {
        self.pre.reset();
        self.post.reset();
    }

    /// Feed the pre-EQ tap. Mono downmix + ring push only; real-time safe.
    #[inline]
    pub fn feed_pre(&self, left: &[f32], right: &[f32]) {
        self.pre.push_stereo(left, right);
    }

    /// Feed the post-EQ tap. Mono downmix + ring push only; real-time safe.
    #[inline]
    pub fn feed_post(&self, left: &[f32], right: &[f32]) {
        self.post.push_stereo(left, right);
    }
}
