//! Crossfade state machine for hot-swapping an audio payload (a NAM
//! model, a convolver, ...) without pops: if a payload is already
//! active it fades out first, the swap lands on the silent sample, and
//! the replacement fades in. With no active payload the replacement is
//! installed immediately and fades in from silence.

/// Active/pending payload pair plus the fade-out/fade-in envelope that
/// masks the handoff. Drive it once per sample via [`SwapFader::next`].
pub struct SwapFader<T> {
    active: Option<T>,
    pending: Option<T>,
    fade_out_remaining: u32,
    fade_in_remaining: u32,
    fade_samples: u32,
    /// Precomputed `1.0 / fade_samples`. LLVM won't fold float division
    /// with a runtime counter, so express the per-sample fade step as a
    /// multiply. Bit-exact with division when `fade_samples` is a power
    /// of two.
    fade_step: f32,
}

impl<T> SwapFader<T> {
    /// `fade_samples` is the length of each fade leg (out and in).
    pub fn new(fade_samples: u32) -> Self {
        debug_assert!(fade_samples > 0);
        Self {
            active: None,
            pending: None,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
            fade_samples,
            fade_step: 1.0 / fade_samples as f32,
        }
    }

    /// Install a payload directly, with no crossfade and no fade-in.
    /// Initialize-time path, before any audio has been processed.
    pub fn install(&mut self, payload: T) {
        self.active = Some(payload);
        self.pending = None;
        self.fade_out_remaining = 0;
        self.fade_in_remaining = 0;
    }

    /// Hand over a freshly loaded payload — starts the swap crossfade.
    /// If a payload is already active it fades out first; otherwise the
    /// new one is swapped in directly and fades in.
    pub fn begin_swap(&mut self, payload: T) {
        self.pending = Some(payload);
        if self.active.is_some() {
            self.fade_out_remaining = self.fade_samples;
            self.fade_in_remaining = 0;
        } else {
            self.active = self.pending.take();
            self.fade_in_remaining = self.fade_samples;
        }
    }

    pub fn active(&self) -> Option<&T> {
        self.active.as_ref()
    }

    pub fn active_mut(&mut self) -> Option<&mut T> {
        self.active.as_mut()
    }

    /// True while the outgoing payload is still fading out (the swap
    /// has not landed yet).
    pub fn is_fading_out(&self) -> bool {
        self.fade_out_remaining > 0
    }

    /// Per-sample tick: returns this sample's fade gain and the payload
    /// it applies to. Performs the swap on the sample where the
    /// fade-out reaches zero, so that (silent) sample and everything
    /// after run through the new payload.
    pub fn next(&mut self) -> (f32, Option<&mut T>) {
        let gain = if self.fade_out_remaining > 0 {
            self.fade_out_remaining -= 1;
            let g = self.fade_out_remaining as f32 * self.fade_step;
            if self.fade_out_remaining == 0 {
                self.active = self.pending.take();
                self.fade_in_remaining = self.fade_samples;
            }
            g
        } else if self.fade_in_remaining > 0 {
            self.fade_in_remaining -= 1;
            1.0 - self.fade_in_remaining as f32 * self.fade_step
        } else {
            1.0
        };
        (gain, self.active.as_mut())
    }
}
