//! Crossfade state machine for hot-swapping an audio payload (a NAM
//! model, a convolver, ...) without pops: if a payload is already
//! active it fades out first, the swap lands on the silent sample, and
//! the replacement fades in. With no active payload the replacement is
//! installed immediately and fades in from silence.
//!
//! # Retirement
//!
//! Swapping displaces the outgoing payload on the audio thread, and for
//! heap-heavy payloads (NAM weight buffers, partitioned-FFT convolvers)
//! dropping it there means a large deallocation — and the allocator
//! lock — inside the sample loop. Attach a retirement sink via
//! [`SwapFader::set_retire_sink`] and displaced payloads are shipped
//! through it (allocation-free `try_send` into a pre-sized channel)
//! instead of being dropped in place; [`SwapFader::spawn_retire_janitor`]
//! provides the matching drop-it-elsewhere thread. If the channel is
//! momentarily full, the payload is parked in one of a few inline slots
//! and re-offered on the next retirement (or via
//! [`SwapFader::take_retired`]). Without a sink the fader keeps its
//! historical drop-in-place behaviour, which is only appropriate for
//! payloads whose `Drop` is trivial (e.g. `SwapFader<f32>`).

use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};

/// How many displaced payloads the fader can park inline when the
/// retirement channel is full. With the consumer swapping at most once
/// per block, at most two payloads can be displaced between blocks (a
/// superseded pending plus a faded-out active), so four slots only fill
/// if the janitor stays unreachable across several consecutive blocks.
const RETIRED_SLOTS: usize = 4;

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
    /// Non-audio-thread destination for displaced payloads. `None` keeps
    /// the historical drop-in-place behaviour.
    retire_tx: Option<SyncSender<T>>,
    /// Inline parking for payloads the sink could not take (channel
    /// full/disconnected). Swept on the next retirement or via
    /// [`SwapFader::take_retired`]; anything still parked when the fader
    /// itself drops is freed with it, off the audio thread.
    retired: [Option<T>; RETIRED_SLOTS],
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
            retire_tx: None,
            retired: [None, None, None, None],
        }
    }

    /// Route displaced payloads to `sink` instead of dropping them in
    /// place. Non-audio-thread setup; pair it with
    /// [`SwapFader::spawn_retire_janitor`] (or any receiver drained off
    /// the audio thread).
    pub fn set_retire_sink(&mut self, sink: SyncSender<T>) {
        self.retire_tx = Some(sink);
    }

    /// Take one payload parked by a full retirement channel, if any.
    /// Lets a consumer without a janitor thread sweep displaced payloads
    /// to its own non-real-time destination.
    pub fn take_retired(&mut self) -> Option<T> {
        self.retired.iter_mut().find_map(Option::take)
    }

    /// Hand a displaced payload off the audio thread. With no sink this
    /// is the historical drop-in-place; with one, the payload (and any
    /// previously parked stragglers) goes out via allocation-free
    /// `try_send`, falling back to the inline parking slots. The only
    /// way a heap payload can still drop here is a sink whose channel
    /// stays full/disconnected until all [`RETIRED_SLOTS`] parking slots
    /// are occupied too — i.e. a dead janitor thread.
    fn retire(&mut self, payload: T) {
        if self.retire_tx.is_none() {
            return;
        }
        self.flush_parked();
        let result = self.retire_tx.as_ref().unwrap().try_send(payload);
        if let Err(TrySendError::Full(p) | TrySendError::Disconnected(p)) = result {
            self.park(p);
        }
    }

    /// Re-offer parked payloads to the sink, stopping at the first one
    /// the channel refuses.
    fn flush_parked(&mut self) {
        let Some(tx) = &self.retire_tx else {
            return;
        };
        for slot in &mut self.retired {
            if let Some(parked) = slot.take() {
                if let Err(TrySendError::Full(p) | TrySendError::Disconnected(p)) =
                    tx.try_send(parked)
                {
                    *slot = Some(p);
                    return;
                }
            }
        }
    }

    fn park(&mut self, payload: T) {
        for slot in &mut self.retired {
            if slot.is_none() {
                *slot = Some(payload);
                return;
            }
        }
        // Every parking slot is taken and the channel is still refusing:
        // the janitor has been unreachable for many blocks. Dropping
        // here — the pre-retirement behaviour — is the last resort.
    }

    /// Install a payload directly, with no crossfade and no fade-in.
    /// Initialize-time path, before any audio has been processed.
    pub fn install(&mut self, payload: T) {
        if let Some(old) = self.active.take() {
            self.retire(old);
        }
        if let Some(old) = self.pending.take() {
            self.retire(old);
        }
        self.active = Some(payload);
        self.fade_out_remaining = 0;
        self.fade_in_remaining = 0;
    }

    /// Hand over a freshly loaded payload — starts the swap crossfade.
    /// If a payload is already active it fades out first; otherwise the
    /// new one is swapped in directly and fades in. A pending payload
    /// superseded by a rapid re-selection is retired, not dropped here.
    ///
    /// A swap requested while a fade is already running never restarts
    /// the envelope at full gain (DSP-07): mid fade-out the fade simply
    /// carries on and the newest payload replaces the pending one; mid
    /// fade-in the fade-out starts from the current gain. So continuous
    /// retargeting (e.g. automated delay time) keeps the gain moving by
    /// at most one fade step per sample, and the latest payload lands.
    pub fn begin_swap(&mut self, payload: T) {
        if let Some(old) = self.pending.take() {
            self.retire(old);
        }
        self.pending = Some(payload);
        if self.active.is_some() {
            if self.fade_out_remaining > 0 {
                // Already fading out: keep going; `pending` was replaced.
                return;
            }
            // Idle (remaining 0 → full length) or mid fade-in, where
            // the last gain was `1 - fade_in_remaining·step`: fade out
            // from there.
            self.fade_out_remaining = self.fade_samples - self.fade_in_remaining;
            self.fade_in_remaining = 0;
            if self.fade_out_remaining == 0 {
                // The fade-in had not emitted a sample yet (gain still
                // 0): land the newest payload right away.
                let old = self.active.take();
                self.active = self.pending.take();
                if let Some(old) = old {
                    self.retire(old);
                }
                self.fade_in_remaining = self.fade_samples;
            }
        } else {
            self.active = self.pending.take();
            self.fade_in_remaining = self.fade_samples;
        }
    }

    /// Fade the active payload out and leave nothing active (an IR
    /// cleared by a preset). A pending payload is dropped (retired); a
    /// fade already running carries on and lands on nothing.
    pub fn begin_clear(&mut self) {
        if let Some(old) = self.pending.take() {
            self.retire(old);
        }
        if self.active.is_none() || self.fade_out_remaining > 0 {
            return;
        }
        self.fade_out_remaining = self.fade_samples - self.fade_in_remaining;
        self.fade_in_remaining = 0;
        if self.fade_out_remaining == 0 {
            if let Some(old) = self.active.take() {
                self.retire(old);
            }
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

    /// True when no fade is in progress: [`Self::next`] would return a
    /// gain of exactly 1.0 and the current active payload for every
    /// sample, so a caller may process a whole block against
    /// [`Self::active_mut`] without ticking.
    pub fn is_settled(&self) -> bool {
        self.fade_out_remaining == 0 && self.fade_in_remaining == 0
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
                let old = self.active.take();
                self.active = self.pending.take();
                if let Some(old) = old {
                    self.retire(old);
                }
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

impl<T: Send + 'static> SwapFader<T> {
    /// Bound of the retirement channel [`SwapFader::spawn_retire_janitor`]
    /// creates. Pre-sized at construction, so the audio-thread `try_send`
    /// never allocates.
    pub const JANITOR_CHANNEL_CAPACITY: usize = 4;

    /// Spawn a janitor thread that does nothing but receive retired
    /// payloads and drop them off the audio thread (the resonance-drums
    /// idiom), returning the sender to hand to
    /// [`SwapFader::set_retire_sink`]. The thread exits once every
    /// sender clone has dropped — i.e. when the fader itself goes away.
    pub fn spawn_retire_janitor(thread_name: &str) -> SyncSender<T> {
        let (tx, rx) = sync_channel(Self::JANITOR_CHANNEL_CAPACITY);
        std::thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(move || {
                // Block on recv; each received payload is dropped here,
                // off the audio thread. Exits when all senders disconnect.
                while rx.recv().is_ok() {}
            })
            .expect("spawn swap-fader janitor thread");
        tx
    }
}
