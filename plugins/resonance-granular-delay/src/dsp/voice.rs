//! Stage 2c/2e — the pitch-synchronous voice path (ba todo #1082, doc
//! #252 §4): the tracker feed, the PSOLA voice pool and its wet buses,
//! and the equal-power crossfade that hands over between the async grain
//! cloud and the voice bus.

use crate::pitch_sync::{PitchSyncGranulator, SpawnMode};
use crate::quantize::{quantize_transpose, PitchQuantize};

use super::grains::GrainBank;
use super::source::SourceRing;
use super::time::{TimeMachine, TimePlan};
use super::BlockParams;

/// Equal-power crossfade length between the async grain cloud and the
/// pitch-synchronous PSOLA bus (ba todo #1082), seconds. Long enough
/// for the fallback cloud to rebuild some overlap, short enough that
/// voiced/unvoiced handovers feel immediate.
pub const VOICE_FADE_SECONDS: f32 = 0.05;

pub(super) struct VoiceStage {
    /// Pitch-synchronous Voice/Mono scheduler (ba todo #1082): tracker,
    /// marker ring and PSOLA voice pool.
    pub(super) psola: PitchSyncGranulator,
    /// PSOLA wet buses, blended into the grain bank's `wet_l`/`wet_r`.
    pub(super) psola_l: Vec<f32>,
    pub(super) psola_r: Vec<f32>,
    /// Mono (mid) scratch of the dry input samples actually written
    /// this block — what the tracker is fed, so marker positions map
    /// 1:1 onto write-stream/buffer positions.
    pub(super) track_in: Vec<f32>,
    /// Equal-power crossfade position between the async cloud (0) and
    /// the PSOLA bus (1); ramps per sample over [`VOICE_FADE_SECONDS`].
    voice_xf: f32,
    /// Last block's engage decision (tracker voiced + usable marker
    /// near the tap) — metering/test aid.
    pub(super) engaged: bool,
}

impl VoiceStage {
    pub(super) fn new(sample_rate: f32, max_block: usize) -> Self {
        Self {
            psola: PitchSyncGranulator::new(sample_rate),
            psola_l: vec![0.0; max_block],
            psola_r: vec![0.0; max_block],
            track_in: vec![0.0; max_block],
            voice_xf: 0.0,
            engaged: false,
        }
    }

    pub(super) fn clear(&mut self) {
        self.psola.reset();
        self.psola_l.fill(0.0);
        self.psola_r.fill(0.0);
        self.voice_xf = 0.0;
        self.engaged = false;
    }

    /// Stage 1c — pitch-sync scheduler state (ba todo #1082, doc #252
    /// §4): feed the tracker the written input, then decide whether
    /// the Voice/Mono path is engaged this block — the tracker must
    /// be voiced AND a stored pitch mark must lie near the delay
    /// tap (so a freshly engaged mode without marker history simply
    /// stays on the async cloud until the buffer has been analysed).
    /// While frozen nothing is fed: the marker ring holds, the last
    /// known period stands, and spawning continues from it against
    /// the stalled head — the frozen drone keeps its pitch lattice.
    /// Returns `(engaged, psola_render)`.
    pub(super) fn resolve(
        &mut self,
        source: &SourceRing,
        time: &TimeMachine,
        sample_rate: f32,
        params: &BlockParams,
        advanced: usize,
        plan: &TimePlan,
    ) -> (bool, bool) {
        let sr = f64::from(sample_rate);
        let mut engaged = false;
        if params.pitch_sync {
            self.psola.sync_to(source.write_pos);
            if advanced > 0 {
                self.psola.feed(&self.track_in[..advanced]);
            }
            let tap_seconds = time.block_tap_seconds(plan);
            let tap_target = source.write_pos as f64 - f64::from(tap_seconds) * sr;
            engaged = self.psola.voiced() && self.psola.has_marker_near(tap_target);
        }
        self.engaged = engaged;
        let psola_render =
            params.pitch_sync || self.voice_xf > 0.0 || self.psola.active_voices() > 0;
        (engaged, psola_render)
    }

    /// Stage 2c-v — pitch-synchronous PSOLA bus (ba todo #1082, doc
    /// #252 §4). Engaged: onsets snap to the pitch mark nearest the
    /// tap, voices are two-period Hann segments at unity rate, and
    /// transposition is onset spacing (period / α) — formants
    /// preserved, no AM beating. Disengaging: the granulator keeps
    /// spawning at the nominal tap with the last known period while
    /// the crossfade drains, so the fallback handover never gaps.
    /// Idle with no live voices it costs nothing (the caller skips it).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render(
        &mut self,
        source: &SourceRing,
        time: &TimeMachine,
        sample_rate: f32,
        frames: usize,
        engaged: bool,
        plan: &TimePlan,
        params: &BlockParams,
        head_adv: f64,
    ) {
        let sr = f64::from(sample_rate);
        self.psola_l[..frames].fill(0.0);
        self.psola_r[..frames].fill(0.0);
        let spawn = if engaged {
            SpawnMode::Marker
        } else if self.voice_xf > 0.0 {
            SpawnMode::Nominal
        } else {
            SpawnMode::None
        };
        // The musical transpose (quantized like the async cloud's)
        // sets the output marker density; per-grain detune spread
        // does not apply to the mono voice lattice.
        let mut semis = params.pitch_semitones;
        if params.quantize != PitchQuantize::Off {
            semis = quantize_transpose(semis, params.quantize, params.scale);
        }
        let alpha = f64::from(semis / 12.0).exp2();
        let tap_seconds = time.block_tap_seconds(plan);
        let delay_samples = f64::from(tap_seconds) * sr;
        let (psl, psr) = (&mut self.psola_l[..frames], &mut self.psola_r[..frames]);
        self.psola.render(
            &source.buf_l,
            &source.buf_r,
            source.write_pos,
            head_adv,
            delay_samples,
            alpha,
            spawn,
            psl,
            psr,
        );
    }

    /// Stage 2e — Voice/Mono blend (ba todo #1082): equal-power
    /// crossfade between the async grain cloud and the PSOLA bus,
    /// ramped per sample over [`VOICE_FADE_SECONDS`], so
    /// voiced/unvoiced handovers (and enabling/disabling the mode)
    /// are transparent. Applied before the feedback stage so
    /// recirculations carry the blended wet.
    pub(super) fn blend(
        &mut self,
        grains: &mut GrainBank,
        sample_rate: f32,
        frames: usize,
        engaged: bool,
    ) {
        let step = 1.0 / (VOICE_FADE_SECONDS * sample_rate).max(1.0);
        let target_xf: f32 = if engaged { 1.0 } else { 0.0 };
        for i in 0..frames {
            if self.voice_xf < target_xf {
                self.voice_xf = (self.voice_xf + step).min(1.0);
            } else if self.voice_xf > target_xf {
                self.voice_xf = (self.voice_xf - step).max(0.0);
            }
            if self.voice_xf > 0.0 {
                let phase = std::f32::consts::FRAC_PI_2 * self.voice_xf;
                let (g_voice, g_async) = phase.sin_cos();
                grains.wet_l[i] = grains.wet_l[i] * g_async + self.psola_l[i] * g_voice;
                grains.wet_r[i] = grains.wet_r[i] * g_async + self.psola_r[i] * g_voice;
            }
        }
    }
}
