//! Click-free FX bypass (ba doc #275 finding X3).
//!
//! Every bypass in the mixer — a whole track / sub-track / bus / master
//! chain, or one individual slot inside a chain — is expressed as a
//! [`BypassFade`]. Toggling one does not switch the signal path on the
//! next sample; it moves a *fade position* that the audio thread walks
//! over [`BYPASS_FADE_MS`] milliseconds, crossfading the processed
//! ("wet") signal against a copy of the chain's / slot's own input
//! ("dry"). Bypassing a reverb therefore fades its tail out over a few
//! milliseconds instead of truncating it, and re-engaging one fades it
//! back in — neither transition can click.
//!
//! # State model
//!
//! - `bypassed` is the *target*, written by the engine thread when a
//!   `SetTrackFxBypass` / `SetBusFxBypass` / `SetMasterFxBypass` /
//!   `SetPluginBypass` command lands.
//! - `pos` is where the fade currently sits, in frames, `0` (fully wet)
//!   ..= `fade_frames(sample_rate)` (fully dry). The audio thread writes
//!   it once per rendered block, from [`BypassFade::stage`];
//!   [`BypassFade::set_bypassed_settled`] also writes it directly, from
//!   the engine thread, to land both without a transition.
//!
//! The two are packed into a single atomic (`BypassFade` itself), not two
//! independent ones: a block reading them while the engine thread settles
//! a bypass must never observe the new target paired with the stale
//! position, or vice versa — see the struct docs and FU-B4a. Reading or
//! writing a bypass state never locks and never allocates, and the fade
//! needs no per-slot heap state at all — the crossfade borrows a
//! pre-allocated [`FxDryScratch`] from the block's scratch set.
//!
//! # Settled vs. fading
//!
//! [`BypassFade::stage`] only advances the fade for the **live** render
//! strategy. Offline renders (bounce, stem export, freeze capture) ask
//! for the *settled* stage instead, so a bounce of a project with a
//! bypassed slot renders that slot bypassed from frame 0 rather than
//! fading it out over the first few milliseconds of the file — and a
//! bounce running next to live playback cannot steal the live fade's
//! position.
//!
//! # Latency
//!
//! A slot the mixer skips entirely contributes no latency, so
//! `latency::slot_latency` drops it from the chain sum and the PDC table
//! is republished (exactly what whole-chain bypass has always done). A
//! slot whose plugin declares its own bypass parameter is *not* skipped —
//! the plugin keeps running and keeps reporting its latency — so
//! bypassing it leaves the comp table completely untouched, which is the
//! alignment-preserving path for latency-carrying plugins. Such a slot is
//! never host-crossfaded either (`PluginSlot::stage`, code review
//! HOST-04): [`run_faded`]'s dry copy is the slot's *undelayed* input,
//! while the plugin's bypassed output is that input delayed by its
//! latency, so the plugin makes its own transition.
//!
//! A whole *chain* bypass on a latent chain still crossfades latent wet
//! against undelayed dry for [`BYPASS_FADE_MS`]; the comp table it moves
//! crossfades every other track to its new alignment over the same
//! length (`LatencyComp::following`, RT-04).

use std::sync::atomic::{AtomicU64, Ordering};

/// Length of every bypass crossfade, in milliseconds. Long enough that
/// even a full-scale polarity flip between wet and dry stays inaudible,
/// short enough that a bypass still feels instant.
pub const BYPASS_FADE_MS: f32 = 5.0;

/// [`BYPASS_FADE_MS`] in frames at `sample_rate`, floored at one frame so
/// the fade always has a non-degenerate length to normalise against.
#[inline]
pub fn fade_frames(sample_rate: u32) -> u32 {
    ((sample_rate as f32 * BYPASS_FADE_MS / 1000.0) as u32).max(1)
}

/// The dry (bypassed) weight at normalised fade position `t`, as a
/// smoothstep.
///
/// Deliberately *equal-gain* (`dry + wet == 1`), not equal-power: dry and
/// wet are the same signal seen through different processing and are
/// therefore strongly correlated, so an equal-power pair would bulge by
/// up to 3 dB mid-fade. Equal gain also makes the fade exactly
/// transparent when the slot happens to be a unity pass-through, which is
/// what lets a bypass transition be asserted sample-accurate in tests.
///
/// Smoothstep rather than a straight line because its derivative is zero
/// at both ends: the fade joins the steady state with no corner, so
/// neither the start nor the end of a transition puts a step in the
/// signal's slope.
#[inline]
pub fn fade_weight(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What one block must do with a chain or slot, given its fade state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FadeStage {
    /// Fully engaged. Process normally; no dry copy, no crossfade — this
    /// is the byte-for-byte unchanged path.
    Wet,
    /// Fully bypassed and settled. The chain / slot is skipped: nothing
    /// is processed and the buffer passes through untouched.
    Dry,
    /// Mid-transition. Process into the buffer, then crossfade it against
    /// the dry copy from `from` to `to` (normalised fade positions).
    Fade { from: f32, to: f32 },
}

impl FadeStage {
    /// True when the caller must snapshot the dry signal before
    /// processing and crossfade against it afterwards.
    #[inline]
    pub fn is_fading(&self) -> bool {
        matches!(self, FadeStage::Fade { .. })
    }
}

/// `(target, position)` packed into the low 33 bits of a `u64`: bit 32 is
/// the target flag, bits 0..32 are the fade position in frames. Packing
/// the pair is the whole fix for FU-B4a — see [`BypassFade`].
const TARGET_BIT: u64 = 1 << 32;

#[inline]
const fn pack(bypassed: bool, pos: u32) -> u64 {
    (if bypassed { TARGET_BIT } else { 0 }) | pos as u64
}

#[inline]
const fn unpack(state: u64) -> (bool, u32) {
    (state & TARGET_BIT != 0, state as u32)
}

/// The bypass state of one chain or one chain slot: a target flag plus
/// the crossfade position the audio thread walks towards it, packed into
/// a single atomic.
///
/// # Why one atomic, not two
///
/// The target and the position must land *together*: [`set_bypassed_settled`]
/// (Self::set_bypassed_settled) restores both to the same end state in one
/// step, and a block concurrently mid-[`stage`](Self::stage) must never be
/// able to observe the new target paired with the old position (or vice
/// versa) — a torn pair like that reads as a genuine, but bogus,
/// in-flight fade, and the audio thread would spend [`BYPASS_FADE_MS`]
/// crossfading against a settle that was supposed to be silent (FU-B4a;
/// `tests/mixer/bypass_settle_race.rs` hammers exactly this). Two separate
/// atomics can't give that guarantee no matter what order they're written
/// in; one `AtomicU64` written with a single `store` (settle) or a single
/// `compare_exchange` (target-only or position-only updates) always can.
#[derive(Debug)]
pub struct BypassFade {
    state: AtomicU64,
}

impl Default for BypassFade {
    fn default() -> Self {
        Self::new()
    }
}

impl BypassFade {
    /// A settled, engaged (not bypassed) state.
    pub const fn new() -> Self {
        Self {
            state: AtomicU64::new(pack(false, 0)),
        }
    }

    /// The current target — what the user asked for, not where the fade
    /// has got to.
    #[inline]
    pub fn bypassed(&self) -> bool {
        unpack(self.state.load(Ordering::Relaxed)).0
    }

    /// Set the target. The rendering thread fades towards it; nothing
    /// switches instantly. Leaves the current position untouched — a
    /// compare-exchange retry loop so a concurrent [`stage`](Self::stage)
    /// advancing the position is never lost.
    #[inline]
    pub fn set_bypassed(&self, v: bool) {
        let mut cur = self.state.load(Ordering::Relaxed);
        loop {
            let (bypassed, pos) = unpack(cur);
            if bypassed == v {
                return;
            }
            match self.state.compare_exchange_weak(
                cur,
                pack(v, pos),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => cur = actual,
            }
        }
    }

    /// Set the target *and* land the fade on it immediately, with no
    /// transition. For state that is being restored rather than changed —
    /// project load / replay — where there is no audio to click.
    ///
    /// One atomic store of the packed pair: a block reading concurrently
    /// through [`stage`](Self::stage) can only ever observe the state from
    /// entirely before this call or entirely after it, never a mix of the
    /// two (FU-B4a).
    pub fn set_bypassed_settled(&self, v: bool) {
        let pos = if v { u32::MAX } else { 0 };
        self.state.store(pack(v, pos), Ordering::Relaxed);
    }

    /// The fade position in frames (clamped to this sample rate's fade
    /// length). Test/diagnostic surface; the render path reads it through
    /// [`stage`](Self::stage).
    pub fn position(&self, sample_rate: u32) -> u32 {
        unpack(self.state.load(Ordering::Relaxed)).1.min(fade_frames(sample_rate))
    }

    /// The settled stage — what this bypass means with no transition in
    /// flight. Used by every offline render.
    #[inline]
    pub fn settled_stage(&self) -> FadeStage {
        if self.bypassed() {
            FadeStage::Dry
        } else {
            FadeStage::Wet
        }
    }

    /// Resolve this block's stage and advance the fade by `frames`.
    ///
    /// `live` is the render strategy's liveness: offline renders get the
    /// settled stage and leave the position untouched (see the module
    /// doc). Must be called at most once per rendered block per fade —
    /// it is the only thing that moves the position.
    ///
    /// Allocation-free and lock-free; safe on the audio thread. Reads and
    /// writes the packed `(target, position)` pair through a
    /// compare-exchange retry loop, so a concurrent [`set_bypassed`]/
    /// [`set_bypassed_settled`] from the control thread is never torn
    /// against and never silently lost.
    pub fn stage(&self, sample_rate: u32, frames: usize, live: bool) -> FadeStage {
        if !live {
            return self.settled_stage();
        }
        let len = fade_frames(sample_rate);
        let step = frames.min(u32::MAX as usize) as u32;
        let mut cur = self.state.load(Ordering::Relaxed);
        loop {
            let (target, raw_pos) = unpack(cur);
            let pos = raw_pos.min(len);
            let goal = if target { len } else { 0 };
            if pos == goal {
                if raw_pos != pos {
                    // Keep a `set_bypassed_settled` sentinel (or a
                    // sample-rate change) from leaving an out-of-range
                    // position behind. Best-effort: if this loses the
                    // race to a concurrent write, that write already
                    // moved the position somewhere valid, so there is
                    // nothing left to correct.
                    let _ = self.state.compare_exchange_weak(
                        cur,
                        pack(target, pos),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                }
                return if target { FadeStage::Dry } else { FadeStage::Wet };
            }
            let next = if target {
                pos.saturating_add(step).min(len)
            } else {
                pos.saturating_sub(step)
            };
            match self.state.compare_exchange_weak(
                cur,
                pack(target, next),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let inv = 1.0 / len as f32;
                    return FadeStage::Fade {
                        from: pos as f32 * inv,
                        to: next as f32 * inv,
                    };
                }
                Err(actual) => cur = actual,
            }
        }
    }
}

/// Pre-allocated dry-signal scratch for the bypass crossfades of one
/// render block.
///
/// Two independent pairs, because a chain-level fade and a slot-level
/// fade can be in flight at the same moment: `chain` holds the input of a
/// whole FX chain, `slot` the input of the individual slot currently
/// being processed inside it. Allocated once by whoever owns the block
/// scratch (the engine's callback buffers, the bounce chunk scratch, the
/// test harness) and only ever borrowed on the audio thread.
pub struct FxDryScratch {
    chain_l: Vec<f32>,
    chain_r: Vec<f32>,
    slot_l: Vec<f32>,
    slot_r: Vec<f32>,
}

#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
impl FxDryScratch {
    /// Allocate for blocks of up to `frames` frames. Allocates — never
    /// call from the audio thread.
    pub fn new(frames: usize) -> Self {
        Self {
            chain_l: vec![0.0; frames],
            chain_r: vec![0.0; frames],
            slot_l: vec![0.0; frames],
            slot_r: vec![0.0; frames],
        }
    }

    /// Borrow both pairs at once: `(chain, slot)`. Disjoint fields, so a
    /// chain crossfade can be staged while its slots run their own.
    #[inline]
    #[allow(clippy::type_complexity)]
    pub fn split(&mut self) -> ((&mut [f32], &mut [f32]), (&mut [f32], &mut [f32])) {
        (
            (&mut self.chain_l, &mut self.chain_r),
            (&mut self.slot_l, &mut self.slot_r),
        )
    }

    /// Frames this scratch can stage a crossfade over.
    #[inline]
    #[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
    pub fn capacity(&self) -> usize {
        self.chain_l.len()
    }
}

/// Snapshot the dry signal before a fading chain / slot processes over
/// it. No-op when the destination is too small (a block longer than the
/// scratch was sized for), which the caller detects through
/// [`can_fade`].
#[inline]
pub fn save_dry(src: (&[f32], &[f32]), dry: (&mut [f32], &mut [f32]), frames: usize) {
    dry.0[..frames].copy_from_slice(&src.0[..frames]);
    dry.1[..frames].copy_from_slice(&src.1[..frames]);
}

/// True when `dry` can stage a crossfade of `frames` frames. A block
/// bigger than the pre-allocated scratch degrades to the settled
/// behaviour rather than allocating on the audio thread.
#[inline]
pub fn can_fade(dry: (&[f32], &[f32]), frames: usize) -> bool {
    dry.0.len() >= frames && dry.1.len() >= frames
}

/// Crossfade a processed buffer towards its dry copy, in place, sweeping
/// the normalised fade position from `from` to `to` across the block.
///
/// Position — not gain — is what interpolates linearly, and
/// [`fade_weight`] shapes it; consecutive blocks therefore chain into one
/// continuous smoothstep, because each block starts at the position the
/// previous one ended on.
#[inline]
pub fn crossfade_to_dry(
    buf: (&mut [f32], &mut [f32]),
    dry: (&[f32], &[f32]),
    frames: usize,
    from: f32,
    to: f32,
) {
    if frames == 0 {
        return;
    }
    let (buf_l, buf_r) = buf;
    let (dry_l, dry_r) = dry;
    let inv = 1.0 / frames as f32;
    for f in 0..frames {
        let t = from + (to - from) * ((f + 1) as f32 * inv);
        let d = fade_weight(t);
        let w = 1.0 - d;
        buf_l[f] = buf_l[f] * w + dry_l[f] * d;
        buf_r[f] = buf_r[f] * w + dry_r[f] * d;
    }
}

/// Run one chain or slot through `process`, honouring `stage` with a
/// click-free crossfade. The single place the bypass contract is
/// implemented — every chain runner (track, sub-track, bus, master,
/// monitor) and every slot inside them goes through it.
///
/// - [`FadeStage::Dry`]: `process` is **not** called and the buffers are
///   left exactly as they arrived — a settled bypass passes audio through
///   bit-for-bit.
/// - [`FadeStage::Wet`]: `process` runs in place with no extra work, so
///   the engaged path is unchanged from before bypass fading existed.
/// - [`FadeStage::Fade`]: the input is copied to `dry`, `process` runs,
///   and the result is swept towards the dry copy.
///
/// Returns whether `process` ran.
#[inline]
pub fn run_faded(
    stage: FadeStage,
    frames: usize,
    buf: (&mut [f32], &mut [f32]),
    dry: (&mut [f32], &mut [f32]),
    process: impl FnOnce(&mut [f32], &mut [f32]) -> bool,
) -> bool {
    let (buf_l, buf_r) = buf;
    let (dry_l, dry_r) = dry;
    match stage {
        FadeStage::Dry => false,
        FadeStage::Wet => process(buf_l, buf_r),
        FadeStage::Fade { from, to } => {
            // A block longer than the pre-allocated dry scratch cannot be
            // staged without allocating, so it falls back to the settled
            // behaviour for this block; the fade position still advanced,
            // so the transition completes on the following blocks.
            if !can_fade((&*dry_l, &*dry_r), frames) {
                return if from >= 1.0 {
                    false
                } else {
                    process(buf_l, buf_r)
                };
            }
            save_dry((&*buf_l, &*buf_r), (dry_l, dry_r), frames);
            let ran = process(buf_l, buf_r);
            crossfade_to_dry(
                (buf_l, buf_r),
                (&dry_l[..frames], &dry_r[..frames]),
                frames,
                from,
                to,
            );
            ran
        }
    }
}
