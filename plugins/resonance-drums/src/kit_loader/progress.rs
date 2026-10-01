//! Kit load progress (drums-plugin-rework.md §5.4, §7 E3).
//!
//! A load reports how many of its sample files are done, and the load is
//! complete only once the **audio thread has taken** the kit it handed
//! off — not when the decode finishes, and not when the kit lands in the
//! mailbox. Everything is atomics, so the audio thread's part
//! ([`KitLoadProgress::note_taken`]) and any reader (the editor, a param
//! read on whatever thread the host likes) never lock or allocate.
//!
//! # How "taken" is known without tagging the kit
//!
//! The mailbox is a one-slot channel, so kits leave it in the order they
//! went in, and each one leaves exactly once: taken by the audio thread
//! (or installed by `initialize`), or reclaimed by a newer hand-off (or
//! discarded by `initialize`). Sends and reclaims happen under
//! `KitBridge::kit_handoff`; a load records the ordinal of its send, and
//! its kit is out of the mailbox once `taken + reclaimed` reaches that
//! ordinal. A reclaim only ever removes a kit a newer load replaced, and
//! the newer load has already restarted the progress by then — so for
//! the load being reported, out of the mailbox means taken.
//!
//! # Whose progress it is
//!
//! Every write names the load (its generation stamp) it is for. `begin`
//! and `idle` move the progress to their load unless a newer one already
//! has it; `handed_off` and `failed` land only if their load is still the
//! one being reported. A loader checks its stamp under `kit_handoff`, but
//! a newer pick can begin after that check and before the old loader's
//! `handed_off` — without the tag the superseded kit would then mark the
//! newer, still decoding, load complete.

use std::sync::atomic::{AtomicU64, Ordering};

const IDLE: u64 = 0;
const DECODING: u64 = 1;
const HANDED: u64 = 2;
const FAILED: u64 = 3;

/// The phase a load is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPhase {
    /// No kit load outstanding: the kit the user wants (or the built-in
    /// one, with none chosen) is what the sampler holds.
    Idle,
    /// Decoding sample files.
    Decoding,
    /// Decoded and handed to the audio thread; complete once it takes it.
    HandedOff,
    /// The load failed; the sampler kept what it had.
    Failed,
}

/// A consistent reading of [`KitLoadProgress`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgressSnapshot {
    pub phase: LoadPhase,
    /// Sample files decoded (or found in the cache) so far, of `files_total`.
    pub files_done: u32,
    /// Sample files the load reads; 0 until it has planned them.
    pub files_total: u32,
    /// The kit is in place on the audio thread.
    pub complete: bool,
}

impl ProgressSnapshot {
    /// 0..1, and 1.0 only when [`complete`](Self::complete): a decoded kit
    /// still waiting for the audio thread reads just under 1. A failed
    /// load reads 0.
    pub fn fraction(&self) -> f32 {
        if self.complete {
            return 1.0;
        }
        match self.phase {
            LoadPhase::Failed => 0.0,
            _ if self.files_total == 0 => 0.0,
            _ => (self.files_done as f32 / self.files_total as f32).min(0.999),
        }
    }
}

/// See the module docs. Shared by the bridge (`KitBridge::load_progress`)
/// and the sampler.
#[derive(Debug, Default)]
pub struct KitLoadProgress {
    /// `phase | generation_tag << 2 | awaited_ordinal << 32`, one word so
    /// a reader sees them together and a writer can compare-and-swap on
    /// the generation: a superseded load's late `handed_off` / `failed`
    /// finds a newer tag and lands nowhere (see [`Self::handed_off`]).
    state: AtomicU64,
    /// `generation << 32 | files_done`, so a superseded loader's late
    /// increments land nowhere.
    done: AtomicU64,
    /// `generation << 32 | files_total`, for the same reason.
    total: AtomicU64,
    sent: AtomicU64,
    reclaimed: AtomicU64,
    taken: AtomicU64,
}

/// Bits of the generation kept in the `state` word's tag.
const TAG_BITS: u32 = 30;
const TAG_MASK: u64 = (1 << TAG_BITS) - 1;

fn tag(generation: u64) -> u64 {
    generation & TAG_MASK
}

fn tag_of(state: u64) -> u64 {
    (state >> 2) & TAG_MASK
}

fn state_word(phase: u64, generation: u64, ordinal: u64) -> u64 {
    phase | (tag(generation) << 2) | ((ordinal & 0xFFFF_FFFF) << 32)
}

/// `generation` is the load tagged in `state`, or a newer one (tags wrap,
/// so "newer" is "less than half the tag space ahead").
fn is_current_or_newer(generation: u64, state: u64) -> bool {
    tag(generation).wrapping_sub(tag_of(state)) & TAG_MASK < (1 << (TAG_BITS - 1))
}

impl KitLoadProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// A load with stamp `generation` starts: nothing done, total unknown.
    /// Ignored if a newer load has already begun.
    pub fn begin(&self, generation: u64) {
        let applied = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                is_current_or_newer(generation, state)
                    .then_some(state_word(DECODING, generation, 0))
            })
            .is_ok();
        if applied {
            let word = (generation & 0xFFFF_FFFF) << 32;
            self.total.store(word, Ordering::Release);
            self.done.store(word, Ordering::Release);
        }
    }

    /// The load `generation` reads `total` files.
    pub fn set_total(&self, generation: u64, total: u32) {
        let tag = generation & 0xFFFF_FFFF;
        let _ = self
            .total
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |word| {
                (word >> 32 == tag).then_some((tag << 32) | total as u64)
            });
    }

    /// One more file of load `generation` is done. A no-op once another
    /// load has begun.
    pub fn file_done(&self, generation: u64) {
        let tag = generation & 0xFFFF_FFFF;
        let _ = self
            .done
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |word| {
                (word >> 32 == tag).then_some(word + 1)
            });
    }

    /// A kit went into the mailbox; its send ordinal. Call under
    /// `kit_handoff`, after the send.
    pub fn note_sent(&self) -> u64 {
        self.sent.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// A kit left the mailbox without reaching the audio thread (taken back
    /// by a newer hand-off, or discarded by `initialize`).
    pub fn note_reclaimed(&self) {
        self.reclaimed.fetch_add(1, Ordering::AcqRel);
    }

    /// Load `generation` handed off the kit sent as `ordinal`. A no-op
    /// unless `generation` is the load the progress is reporting: a
    /// superseded load that got as far as its hand-off must not mark
    /// complete while a newer one decodes.
    pub fn handed_off(&self, generation: u64, ordinal: u64) {
        self.finish(generation, state_word(HANDED, generation, ordinal));
    }

    /// Load `generation` failed. A no-op once another load has begun.
    pub fn failed(&self, generation: u64) {
        self.finish(generation, state_word(FAILED, generation, 0));
    }

    /// A kit that is no loader's (a direct `hand_off_kit`) was sent as
    /// `ordinal`: whatever load the progress was reporting, the sampler
    /// now gets this kit.
    pub fn handed_off_directly(&self, ordinal: u64) {
        let _ = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                Some(state_word(HANDED, tag_of(state), ordinal))
            });
    }

    fn finish(&self, generation: u64, word: u64) {
        let _ = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (tag_of(state) == tag(generation)).then_some(word)
            });
    }

    /// Nothing is loading: what the sampler holds is what is wanted.
    /// `generation` is the newest load stamp the caller knows of; a load
    /// begun after it keeps its progress.
    pub fn idle(&self, generation: u64) {
        let _ = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                is_current_or_newer(generation, state).then_some(state_word(IDLE, generation, 0))
            });
    }

    /// Audio thread (and `initialize`'s install): a kit was taken from the
    /// mailbox. One atomic add — no lock, no allocation.
    #[inline]
    pub fn note_taken(&self) {
        self.taken.fetch_add(1, Ordering::AcqRel);
    }

    /// Kits the audio thread has taken so far.
    pub fn kits_taken(&self) -> u64 {
        self.taken.load(Ordering::Acquire)
    }

    /// Read the progress. Lock-free; retries if a load changed phase
    /// mid-read.
    pub fn snapshot(&self) -> ProgressSnapshot {
        loop {
            let state = self.state.load(Ordering::Acquire);
            let done = self.done.load(Ordering::Acquire) as u32;
            let total = self.total.load(Ordering::Acquire) as u32;
            let out = self.taken.load(Ordering::Acquire) + self.reclaimed.load(Ordering::Acquire);
            if self.state.load(Ordering::Acquire) != state {
                continue;
            }
            // Ordinals are kept to 32 bits in the state word; compare
            // modulo that.
            let awaited = (state >> 32) as u32;
            let reached = (out as u32).wrapping_sub(awaited) < (1 << 31);
            let (phase, complete) = match state & 3 {
                IDLE => (LoadPhase::Idle, true),
                DECODING => (LoadPhase::Decoding, false),
                HANDED => (LoadPhase::HandedOff, reached),
                _ => (LoadPhase::Failed, false),
            };
            return ProgressSnapshot {
                phase,
                files_done: done,
                files_total: total,
                complete,
            };
        }
    }

    /// [`ProgressSnapshot::fraction`] of a fresh snapshot: the value K4's
    /// `kit_load_progress` param reports.
    pub fn fraction(&self) -> f32 {
        self.snapshot().fraction()
    }

    /// The kit the user wants is in place on the audio thread.
    pub fn is_complete(&self) -> bool {
        self.snapshot().complete
    }
}
