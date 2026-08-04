//! Rhythmic gating and ducking of the delay's wet signal.
//!
//! Both shape the wet path only, *after* the delay tap and before the
//! wet/dry mix — the feedback path never sees them. That distinction is
//! the whole point: chopping inside the loop would re-record the chopped
//! signal and the repeats would decay into a stutter, whereas gating the
//! output leaves the tail intact and simply lets it through in a rhythm.
//!
//! **Gate** is a tempo-synced square window with soft edges: the wet
//! opens for `width` of each period and closes to `1 − depth` for the
//! rest, with `edge`-long ramps so the transitions don't click.
//!
//! **Duck** pulls the wet down while the dry input is loud — the "delay
//! gets out of the way of the vocal, then blooms in the gaps" effect. It
//! is a real log-domain ducker built on the shared
//! [`resonance_dsp::dynamics`] primitives (the same detector → threshold →
//! ballistics topology the compressors use), keyed off this plugin's own
//! dry input, so it needs no host-side sidechain routing.

use resonance_dsp::dynamics::{soft_knee_gain_reduction_db, Ballistics};

/// Attack time of the ducker, in milliseconds. Fixed rather than exposed:
/// a ducking delay always wants the wet out of the way *now* and only the
/// recovery is a musical choice, which is what `duck_release` is for.
pub const DUCK_ATTACK_MS: f32 = 5.0;

/// Gain reduction at `duck_amount = 1.0`, in dB.
pub const DUCK_MAX_GR_DB: f32 = 24.0;

/// Knee width of the ducker's gain computer, in dB.
const DUCK_KNEE_DB: f32 = 6.0;

/// Gain applied to the wet signal at gate phase `phase`.
///
/// `phase` runs `0.0..1.0` across one gate period. The window opens at
/// phase 0 for `width` of the period; `edge` is the ramp length as a
/// fraction of the period, clamped so both ramps always fit inside the
/// open and closed halves (a wide edge on a narrow window degrades to a
/// triangle rather than mis-shaping the window). `depth` is how far the
/// closed phase attenuates: `1.0` is silent, `0.0` is no gating at all.
///
/// Pure so the window shape can be unit-tested without a delay line.
pub fn gate_gain(phase: f32, width: f32, edge: f32, depth: f32) -> f32 {
    let width = width.clamp(0.0, 1.0);
    let phase = phase - phase.floor();
    // Ramps must fit in the open window and in the closed remainder;
    // below 1e-6 the ramp is a step (and avoids a divide by zero).
    let e = edge
        .min(width * 0.5)
        .min((1.0 - width) * 0.5)
        .max(1e-6);
    let open = if phase >= width {
        0.0
    } else if phase < e {
        phase / e
    } else if phase < width - e {
        1.0
    } else {
        (width - phase) / e
    };
    1.0 - depth.clamp(0.0, 1.0) * (1.0 - open.clamp(0.0, 1.0))
}

/// One gate period in samples, from the tempo and a division index into
/// [`crate::sync::DIVISION_LABELS`]. Falls back to one second when the
/// host reports no tempo, so an unsynced host still gates at a sane rate
/// instead of dividing by zero.
pub fn gate_period_samples(
    division: usize,
    tempo: Option<resonance_plugin::TempoInfo>,
    sample_rate: f32,
) -> f32 {
    let sample_rate = sample_rate.max(1.0);
    match tempo {
        Some(t) => {
            let samples_per_beat = 60.0 / t.bpm.max(20.0) * sample_rate;
            (samples_per_beat * crate::sync::division_beats(division)).max(1.0)
        }
        None => sample_rate,
    }
}

/// Per-block gate + duck settings, resolved once in `process`.
pub struct GateDuckParams {
    pub gate_on: bool,
    /// Length of one gate period in samples.
    pub gate_period: f32,
    pub gate_width: f32,
    /// Edge ramp as a fraction of the period.
    pub gate_edge: f32,
    pub gate_depth: f32,
    /// `0.0` disables ducking entirely.
    pub duck_amount: f32,
    pub duck_threshold_db: f32,
    pub duck_release_ms: f32,
}

/// Gate phase + ducker envelope, carried across blocks.
pub struct GateDuck {
    sample_rate: f32,
    /// Gate position, `0.0..1.0` through the current period.
    phase: f32,
    /// Smoothed ducking gain reduction, in dB (non-negative).
    gr_db: f32,
    ballistics: Ballistics,
    release_ms: f32,
}

impl GateDuck {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0),
            phase: 0.0,
            gr_db: 0.0,
            ballistics: Ballistics::from_times(sample_rate, DUCK_ATTACK_MS, 200.0),
            release_ms: 200.0,
        }
    }

    pub fn clear(&mut self) {
        self.phase = 0.0;
        self.gr_db = 0.0;
    }

    /// Refresh the ducker's ballistics when the release time changed.
    /// Recomputing exp coefficients per sample would be wasteful, and per
    /// block is inaudible for a release control.
    pub fn prepare_block(&mut self, params: &GateDuckParams) {
        if (params.duck_release_ms - self.release_ms).abs() > f32::EPSILON {
            self.release_ms = params.duck_release_ms;
            self.ballistics =
                Ballistics::from_times(self.sample_rate, DUCK_ATTACK_MS, params.duck_release_ms);
        }
    }

    /// Advance one sample and return the gain to apply to the wet signal.
    /// `dry_l`/`dry_r` are this sample's DRY input — the ducker keys off
    /// the incoming signal, not off its own output.
    #[inline]
    pub fn next_gain(&mut self, dry_l: f32, dry_r: f32, params: &GateDuckParams) -> f32 {
        let gate = if params.gate_on {
            let g = gate_gain(
                self.phase,
                params.gate_width,
                params.gate_edge,
                params.gate_depth,
            );
            self.phase += 1.0 / params.gate_period.max(1.0);
            self.phase -= self.phase.floor();
            g
        } else {
            // Keep the phase parked at the top so re-enabling the gate
            // starts on an open window rather than mid-chop.
            self.phase = 0.0;
            1.0
        };

        if params.duck_amount <= 0.0 {
            // Let any residual reduction recover rather than snapping the
            // wet back to full the moment the amount reaches zero.
            self.gr_db = self.ballistics.step_envelope(self.gr_db, 0.0);
            return gate * db_to_linear(-self.gr_db);
        }

        let detector = dry_l.abs().max(dry_r.abs());
        let detector_db = linear_to_db(detector);
        // Slope 1.0 = infinite ratio: everything above the threshold turns
        // into reduction, capped at the amount-scaled maximum. A ducker
        // wants a hard hand-off, not a compression curve.
        let raw_gr = soft_knee_gain_reduction_db(
            detector_db,
            params.duck_threshold_db,
            DUCK_KNEE_DB,
            DUCK_KNEE_DB * 0.5,
            1.0,
        );
        let target = raw_gr.min(params.duck_amount * DUCK_MAX_GR_DB);
        self.gr_db = self.ballistics.step_envelope(self.gr_db, target);
        gate * db_to_linear(-self.gr_db)
    }
}

#[inline]
fn linear_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

#[inline]
fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
