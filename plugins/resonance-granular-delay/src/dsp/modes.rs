//! The DSP core's mode enums: what the Time, Scheduler, FB Route,
//! Damping and Quality choices mean to the render path (doc #252
//! §3/§4/§5/§9).
//!
//! They live together, away from the render stages, because every stage
//! matches on them and none of them owns one. Each one carries its own
//! index ↔ variant ↔ label mapping via [`choice_param!`] (ba todo
//! #1267), so `params.rs`' declared range, `lib.rs`' resolution and the
//! editor's label list cannot drift apart.

use resonance_dsp::{SchedulerMode, MAX_GRAINS};

use crate::choice::choice_param;

/// Reduced grain-pool cap of the Lo-fi tier (ba todo #1083; doc #252
/// §2 — Clouds' grain count shrinks on its low-quality modes). Half
/// the [`MAX_GRAINS`] pool: sparse enough to be part of the lo-fi
/// character, dense enough to stay a cloud.
pub const LOFI_MAX_GRAINS: usize = MAX_GRAINS / 2;

/// Quality tier (ba todo #1083, doc #252 §3/§9): interpolation order,
/// anti-aliasing, µ-law lo-fi character and grain-pool size.
///
/// Every tier ingredient is grain-latched at spawn (kernel, µ-law
/// flag) or click-free by construction (the pool cap steals through
/// the release ramp; the AA one-pole is per grain), so switching tiers
/// mid-stream produces no discontinuity — sounding grains finish
/// exactly as they spawned, and no buffer format ever changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QualityTier {
    /// 2-point linear reads, 8-bit µ-law quantization of every grain
    /// stream, pool capped at [`LOFI_MAX_GRAINS`] — the Clouds-style
    /// lo-fi character.
    LoFi,
    /// 4-point Hermite reads, no anti-aliasing — the pre-#1083
    /// behaviour, bit-identical.
    #[default]
    Normal,
    /// 6-point Lagrange reads (interpolating, flatter and cleaner than
    /// Hermite, see `resonance_dsp::read_lagrange6_wrapped`) plus the rate-tracked per-grain
    /// anti-alias lowpass forced on for upward transposition.
    Hq,
}

choice_param!(QualityTier, fallback: QualityTier::Normal, {
    QualityTier::LoFi => "Lo-Fi",
    QualityTier::Normal => "Norm",
    QualityTier::Hq => "HQ",
});

/// Delay-time change behaviour (doc #252 §5, ba todo #1076).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeMode {
    /// Dual-tap swap via [`resonance_dsp::SwapFader`]: on a time change
    /// the whole granulated wet fades out over
    /// [`super::FADE_LEG_SECONDS`], the tap jumps on the silent sample
    /// (every in-flight grain is retired there, click-free by
    /// construction) and the new origin fades back in.
    Fade,
    /// Tape/BBD: the effective delay slews toward the target (one-pole,
    /// [`super::REPITCH_TAU_SECONDS`]) and grains spawned during the
    /// slew take the matching playback-rate offset `1 − d(delay)/dt` —
    /// the tape-style momentary pitch swoop, settling back to unity.
    Repitch,
    /// Default, uniquely granular: a time change affects only newly
    /// spawned grains; in-flight grains finish at their old origin —
    /// artifact-free by construction.
    PerGrain,
}

choice_param!(TimeMode, fallback: TimeMode::PerGrain, {
    TimeMode::Fade => "Fade",
    TimeMode::Repitch => "Repitch",
    TimeMode::PerGrain => "Grain",
});

/// Grain-onset scheduler (doc #252 §4/§9, ba todo #1082): how the
/// async cloud is clocked, and whether the pitch-synchronous PSOLA
/// voice path runs on top of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheduler {
    /// Regular onsets at exactly the density interval.
    Sync,
    /// Default: randomized inter-onset times around the density
    /// interval (doc #252 §9).
    #[default]
    Async,
    /// Pitch-Sync: the tracker runs on the written input and, while it
    /// is voiced, onsets snap to pitch marks (PSOLA voices). The
    /// engines keep running [`Scheduler::Async`] underneath as the
    /// unvoiced/unlocked fallback.
    PitchSync,
}

choice_param!(Scheduler, fallback: Scheduler::Async, {
    Scheduler::Sync => "Sync",
    Scheduler::Async => "Async",
    Scheduler::PitchSync => "Voice",
});

impl Scheduler {
    /// The engine-side onset mode this choice runs the async cloud at
    /// (Pitch-Sync falls back to Async grains under the voice bus).
    pub fn engine_mode(self) -> SchedulerMode {
        match self {
            Scheduler::Sync => SchedulerMode::Sync,
            Scheduler::Async | Scheduler::PitchSync => SchedulerMode::Async,
        }
    }

    /// Whether the pitch-synchronous voice path is enabled.
    pub fn pitch_sync(self) -> bool {
        matches!(self, Scheduler::PitchSync)
    }
}

/// Damping filter type in the feedback loop (doc #252 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DampingFilter {
    /// Default: one-pole lowpass — repeats darken as they recirculate.
    #[default]
    Lowpass,
    /// Highpass, realized as input-minus-lowpass so the one-pole state
    /// stays valid across type switches.
    Highpass,
}

choice_param!(DampingFilter, fallback: DampingFilter::Lowpass, {
    DampingFilter::Lowpass => "LP",
    DampingFilter::Highpass => "HP",
});

impl DampingFilter {
    /// The flag the feedback chain consumes.
    pub fn is_highpass(self) -> bool {
        matches!(self, DampingFilter::Highpass)
    }
}

/// Feedback topology (doc #252 §1, ba todo #1074).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbRoute {
    /// Default: the granulated wet output is filtered, soft-clipped,
    /// DC-blocked and summed with the dry input at the buffer write
    /// point, so every recirculation is re-granulated (the defining
    /// granular-delay sound).
    WetToBuffer,
    /// "Clean repeats": the buffer receives the dry input only; the
    /// feedback recirculates in the *output* mix through a dedicated
    /// wet-recirculation ring read at the delay time.
    OutputOnly,
    /// Wet→Buffer with the channels crossed at the feedback write tap
    /// (ba todo #1077, doc #252 §5): each recirculation the conditioned
    /// left wet feeds the right buffer input and vice versa, so repeats
    /// alternate sides.
    PingPong,
}

// The fallback is Output-only, not the factory default: that is the
// route the plugin's original catch-all arm resolved out-of-range
// indices to, and `IntParam::set_value` does not clamp.
choice_param!(FbRoute, fallback: FbRoute::OutputOnly, {
    FbRoute::WetToBuffer => "Wet→Buf",
    FbRoute::OutputOnly => "Out Only",
    FbRoute::PingPong => "Pong",
});

impl FbRoute {
    /// Whether this route writes the conditioned wet back into the
    /// grain source buffer (so every recirculation is re-granulated).
    pub(super) fn feeds_buffer(self) -> bool {
        matches!(self, FbRoute::WetToBuffer | FbRoute::PingPong)
    }
}
