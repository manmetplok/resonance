//! Decay of PROGRAM MATERIAL: the reverberation time of a mix slice, a
//! track or a reverb return, read off a stop in the music instead of an
//! impulse response (reverb-algorithms.md R9, the `meter.measure`
//! `decay` detail).
//!
//! ## Method
//!
//! This is the interrupted-noise method (ISO 3382-2 §5.3) with music as
//! the noise: when a sustained signal stops, what is left is the room's
//! free decay, and the decay times fitted to it are the room's. The steps:
//!
//! 1. Envelope: `L² + R²` averaged over 10 ms windows, then smoothed
//!    over 50 ms (centred), in dB.
//! 2. A window is *sustained* when it is within [`SUSTAIN_TOLERANCE_DB`]
//!    of the loudest window in the [`SUSTAIN_LOOKBACK_S`] before it, and
//!    above the gate (−100 dB absolute and within 60 dB of the range's
//!    loudest window). A *stop* is the last sustained window of a
//!    stretch: the next one has started to fall.
//! 3. From each stop the decay runs until the next onset (a rise of
//!    [`ONSET_RISE_DB`] over the lowest level since the stop) or the end
//!    of the buffer. Its dynamic range is the plateau level (the median
//!    of the half second up to the stop) minus the lowest level reached.
//!    The decay starts where a line fitted to its first 15 dB meets the
//!    plateau level, since on a slow decay the stop is detected late.
//! 4. The analysed decay is the LATEST stop whose decay is clean: it
//!    falls far enough for T30 (see [`required_range_db`]), i.e. the
//!    song's last note, or the last break in the arrangement. When none
//!    does, the deepest decay is reported, marked not clean, with
//!    whatever times its range supports.
//! 5. The decay's energy is Schroeder-integrated with the impulse
//!    harness ([`edc_from_energy`]: onset, Lundeby noise-floor truncation,
//!    EDT / T20 / T30), broadband and per octave band.
//!
//! For a steady excitation the free decay is already the backward
//! integral of the response's energy, and integrating it again keeps an
//! exponential's slope, so T20 / T30 read the room's T60.
//!
//! ## Limits
//!
//! - A decay overlapped by new notes is not a decay. The finder cuts it
//!   at the next onset, so a tail that the next note covers before it
//!   has fallen 35 dB has no T30, and one that ANOTHER track keeps
//!   covering (a pad still playing under the stop) never reads as a
//!   decay at all: measure the one track, or a range where the whole
//!   arrangement stops.
//! - A source that decays on its own (a piano note, a plucked string)
//!   is not sustained, so its own decay counts: the stop is its attack,
//!   and EDT mixes the note's decay with the room's. Sustained material
//!   (pads, held notes, noise) gives clean readings.
//! - Dry signal on the target makes the stop a cliff before the tail: EDT
//!   reads short against T30, which (fitted from −5 dB) still reads the
//!   room once the wet tail dominates.
//! - A tail longer than about 15 s falls under 2 dB per 500 ms, which
//!   step 2 cannot tell from a sustained level, so it finds no stop.
//! - The range must contain the tail: a decay still falling when the
//!   range ends has `t30 == None` (and `ends == RangeEnd`).

use super::bands::{OctaveBandFilter, OCTAVE_BANDS_HZ};
use super::edc::{edc_from_energy, DecayTimes};
use super::BandDecay;

/// Envelope window, seconds.
pub const ENVELOPE_WINDOW_S: f32 = 0.010;
/// A window is sustained when it is within this many dB of the loudest
/// window in the [`SUSTAIN_LOOKBACK_S`] before it.
pub const SUSTAIN_TOLERANCE_DB: f32 = 2.0;
/// See [`SUSTAIN_TOLERANCE_DB`].
pub const SUSTAIN_LOOKBACK_S: f32 = 0.5;
/// A rise this far over the lowest level since the stop is a new onset.
pub const ONSET_RISE_DB: f32 = 6.0;
/// The bottom of T30's fit range (−5 … −35 dB).
pub const CLEAN_DECAY_DB: f32 = 35.0;
/// Range a decay needs below the bottom of a fit, dB: 5 when it ends on a
/// floor (the Lundeby truncation handles that), 10 when it is cut off by
/// an onset or the range end, where nothing estimates the missing energy
/// and plain backward integration bends the curve's last few dB.
pub const FLOOR_MARGIN_DB: f32 = 5.0;
/// See [`FLOOR_MARGIN_DB`].
pub const CUT_MARGIN_DB: f32 = 10.0;

/// The dynamic range a decay ending in `ends` needs for a fit that ends
/// `fit_end_db` below the plateau (10 for EDT, 25 for T20, 35 for T30).
pub fn required_range_db(fit_end_db: f32, ends: DecayEnd) -> f32 {
    fit_end_db
        + match ends {
            DecayEnd::Floor => FLOOR_MARGIN_DB,
            DecayEnd::Onset | DecayEnd::RangeEnd => CUT_MARGIN_DB,
        }
}
/// Dynamic ranges are capped here; a decay into digital silence reads it.
pub const MAX_DYNAMIC_RANGE_DB: f32 = 100.0;
/// The level a tail must fall below its plateau for
/// [`ProgramDecay::tail_20db_s`].
pub const TAIL_DROP_DB: f32 = 20.0;

/// Smoothing radius over the 10 ms windows (5 windows = 50 ms, centred).
const SMOOTH_RADIUS: usize = 2;
/// Absolute gate, dB of mean `L² + R²` (a full-scale sine on both
/// channels reads 0).
const GATE_ABS_DB: f32 = -100.0;
/// Gate below the range's loudest smoothed window, dB.
const GATE_REL_DB: f32 = 60.0;
/// The knee is found by fitting a line to the first this-many dB of the
/// decay and extrapolating it back to the plateau level.
const KNEE_FIT_DB: f32 = 15.0;
/// The knee is no earlier than the last window within this of the
/// plateau level.
const KNEE_HOLD_DB: f32 = 0.5;
/// Pre-roll the octave filters run on before the decay starts, seconds,
/// so their own start-up transient is over by then.
const BAND_PREROLL_S: f32 = 0.2;
/// A floor: the level has stayed within this of its minimum for the
/// last [`FLOOR_HOLD_S`] of the decay.
const FLOOR_FLAT_DB: f32 = 1.5;
/// See [`FLOOR_FLAT_DB`].
const FLOOR_HOLD_S: f32 = 0.3;

/// How an analysed decay ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecayEnd {
    /// It reached a floor (digital silence or a steady noise floor) and
    /// stayed there.
    Floor,
    /// New signal arrived: a note, or another part coming back in.
    Onset,
    /// The buffer ended while it was still falling.
    RangeEnd,
}

/// The decay found in a stretch of program material. See the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramDecay {
    /// A stop was found at all. When `false` every other figure is empty.
    pub found: bool,
    /// The decay falls far enough for T30 ([`required_range_db`]).
    pub clean: bool,
    /// First sample of the analysed decay, from the buffer's start.
    pub start: usize,
    /// One past its last sample.
    pub end: usize,
    /// Why it ends where it does. `None` when nothing was found.
    pub ends: Option<DecayEnd>,
    /// Plateau level before the stop minus the lowest level of the decay,
    /// dB, capped at [`MAX_DYNAMIC_RANGE_DB`]. 0 when nothing was found.
    pub dynamic_range_db: f32,
    /// Broadband EDT / T20 / T30 of `L² + R²`, seconds; each `None` when
    /// the decay does not reach its fit range.
    pub times: DecayTimes,
    /// T30 (and EDT / T20) per octave band, 125 Hz – 8 kHz.
    pub bands: [BandDecay; 7],
    /// Seconds from `start` until the level is [`TAIL_DROP_DB`] below the
    /// plateau, `None` when it never gets there inside the decay.
    pub tail_20db_s: Option<f32>,
    /// The sample rate the sample positions count in.
    pub sample_rate: f32,
}

impl ProgramDecay {
    fn none(sample_rate: f32) -> Self {
        Self {
            found: false,
            clean: false,
            start: 0,
            end: 0,
            ends: None,
            dynamic_range_db: 0.0,
            times: DecayTimes::default(),
            bands: OCTAVE_BANDS_HZ.map(|center_hz| BandDecay {
                center_hz,
                times: DecayTimes::default(),
            }),
            tail_20db_s: None,
            sample_rate,
        }
    }

    /// `start` in seconds from the buffer's start.
    pub fn start_s(&self) -> f32 {
        self.start as f32 / self.sample_rate
    }

    /// Length of the analysed decay, seconds.
    pub fn length_s(&self) -> f32 {
        self.end.saturating_sub(self.start) as f32 / self.sample_rate
    }

    /// Why the result is not a clean decay, in a sentence; `None` when it
    /// is one. Times are seconds from the buffer's start plus `offset_s`
    /// (the buffer's own start on the caller's timeline).
    pub fn note(&self, offset_s: f64) -> Option<String> {
        if !self.found {
            return Some(
                "no stop found: the level never falls away from a sustained level in this \
                 range (a tail that something else keeps covering is not a decay)"
                    .into(),
            );
        }
        if self.clean {
            return None;
        }
        let why = match self.ends {
            Some(DecayEnd::Onset) => format!(
                "new signal comes in at {:.2} s",
                offset_s + self.end as f64 / self.sample_rate as f64
            ),
            Some(DecayEnd::RangeEnd) => "the range ends".into(),
            _ => "it levels off (a noise floor, or a part that keeps playing)".into(),
        };
        Some(format!(
            "no clean decay: the deepest one, from {:.2} s, falls only {:.0} dB before {why}; \
             T30 needs {:.0} dB (measure a range that ends in silence after a stop)",
            offset_s + self.start_s() as f64,
            self.dynamic_range_db,
            required_range_db(CLEAN_DECAY_DB, self.ends.unwrap_or(DecayEnd::RangeEnd)),
        ))
    }
}

fn db(e: f64) -> f32 {
    (10.0 * e.max(1e-30).log10()) as f32
}

/// One candidate decay, in window indices.
struct Candidate {
    /// The stop (last sustained smoothed window).
    stop: usize,
    /// One past the decay's last window.
    end: usize,
    ends: DecayEnd,
    plateau_db: f32,
    drop_db: f32,
}

/// Find and measure the last free decay in a stereo buffer. See the
/// module docs for the method and its limits.
pub fn program_decay(left: &[f32], right: &[f32], sample_rate: f32) -> ProgramDecay {
    let n = left.len().min(right.len());
    let (left, right) = (&left[..n], &right[..n]);
    let win = ((ENVELOPE_WINDOW_S * sample_rate).round() as usize).max(1);
    let energy: Vec<f64> = left
        .iter()
        .zip(right)
        .map(|(&l, &r)| (l as f64) * (l as f64) + (r as f64) * (r as f64))
        .collect();
    let raw: Vec<f64> = energy
        .chunks(win)
        .map(|c| c.iter().sum::<f64>() / c.len() as f64)
        .collect();
    let windows = raw.len();
    if windows < 3 {
        return ProgramDecay::none(sample_rate);
    }
    let smooth: Vec<f32> = (0..windows)
        .map(|i| {
            let lo = i.saturating_sub(SMOOTH_RADIUS);
            let hi = (i + SMOOTH_RADIUS + 1).min(windows);
            db(raw[lo..hi].iter().sum::<f64>() / (hi - lo) as f64)
        })
        .collect();
    
    let loudest = smooth.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let gate = GATE_ABS_DB.max(loudest - GATE_REL_DB);
    let lookback = ((SUSTAIN_LOOKBACK_S / ENVELOPE_WINDOW_S).round() as usize).max(1);
    let sustained: Vec<bool> = (0..windows)
        .map(|i| {
            let recent = smooth[i.saturating_sub(lookback)..=i]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            smooth[i] >= gate && smooth[i] >= recent - SUSTAIN_TOLERANCE_DB
        })
        .collect();

    let hold = ((FLOOR_HOLD_S / ENVELOPE_WINDOW_S).round() as usize).max(1);
    let candidate = |stop: usize| -> Candidate {
        let mut low = smooth[stop];
        let mut end = windows;
        let mut ends = DecayEnd::RangeEnd;
        for k in stop + 1..windows {
            if smooth[k] > low + ONSET_RISE_DB && smooth[k] >= gate {
                // The smoothing reaches SMOOTH_RADIUS windows ahead of the
                // onset; cut before it.
                end = k.saturating_sub(SMOOTH_RADIUS).max(stop + 1);
                ends = DecayEnd::Onset;
                break;
            }
            low = low.min(smooth[k]);
        }
        // The plateau: the median smoothed level of the sustained
        // look-back, which a stop detected a little late into the decay
        // does not drag down.
        let mut recent: Vec<f32> = smooth[stop.saturating_sub(lookback)..=stop].to_vec();
        recent.sort_by(f32::total_cmp);
        let plateau_db = recent[recent.len() / 2];
        let low = smooth[stop + 1..end].iter().copied().fold(smooth[stop], f32::min);
        if ends == DecayEnd::RangeEnd {
            let tail_from = end.saturating_sub(hold).max(stop + 1);
            let tail = &smooth[tail_from..end];
            let top = tail.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let flat = end - tail_from >= hold && top - low <= FLOOR_FLAT_DB;
            if flat || plateau_db - low >= MAX_DYNAMIC_RANGE_DB {
                ends = DecayEnd::Floor;
            }
        }
        Candidate {
            stop,
            end,
            ends,
            plateau_db,
            drop_db: (plateau_db - low).clamp(0.0, MAX_DYNAMIC_RANGE_DB),
        }
    };

    // Latest clean stop first; else the deepest (latest on a tie).
    let mut best: Option<Candidate> = None;
    for stop in (0..windows - 1).rev() {
        if !(sustained[stop] && !sustained[stop + 1]) {
            continue;
        }
        let c = candidate(stop);
        if c.drop_db >= required_range_db(CLEAN_DECAY_DB, c.ends) {
            best = Some(c);
            break;
        }
        if best.as_ref().is_none_or(|b| c.drop_db > b.drop_db) {
            best = Some(c);
        }
    }
    let Some(c) = best else {
        return ProgramDecay::none(sample_rate);
    };

    // The knee: a line fitted to the first KNEE_FIT_DB of the decay,
    // extrapolated back to the plateau. The stop itself is up to
    // SUSTAIN_TOLERANCE_DB late on a slow decay.
    let centre = |k: usize| (k as f64 + 0.5) * win as f64;
    let pts: Vec<(f64, f64)> = (c.stop + 1..c.end)
        .take_while(|&k| smooth[k] >= c.plateau_db - KNEE_FIT_DB)
        .map(|k| (centre(k), smooth[k] as f64))
        .collect();
    // Never before the last window still on the plateau: a cliff (dry
    // signal stopping) followed by a slow tail fits a shallow line that
    // would otherwise meet the plateau well before the stop.
    let earliest = (c.stop.saturating_sub(lookback)..=c.stop)
        .rev()
        .find(|&k| smooth[k] >= c.plateau_db - KNEE_HOLD_DB)
        .map_or(c.stop.saturating_sub(lookback) as f64 * win as f64, centre);
    let knee = match fit_line(&pts) {
        Some((a, b)) if b < 0.0 => ((c.plateau_db as f64 - a) / b).clamp(earliest, centre(c.stop)),
        _ => centre(c.stop),
    };
    let start = (knee.round() as usize).min(n);
    let end = (c.end * win).min(n).max(start);

    let mut out = ProgramDecay::none(sample_rate);
    out.found = true;
    out.clean = c.drop_db >= required_range_db(CLEAN_DECAY_DB, c.ends);
    out.start = start;
    out.end = end;
    out.ends = Some(c.ends);
    out.dynamic_range_db = c.drop_db;
    out.tail_20db_s = (c.stop + 1..c.end)
        .find(|&k| smooth[k] <= c.plateau_db - TAIL_DROP_DB)
        .map(|k| ((centre(k) - start as f64).max(0.0) / sample_rate as f64) as f32);
    if end <= start + 1 {
        return out;
    }
    // A fit whose range the decay does not cover is no reading, whatever
    // the curve's cut-off end would fit to.
    let reach = |fit_end_db: f32| c.drop_db >= required_range_db(fit_end_db, c.ends);
    let gated = |t: DecayTimes| DecayTimes {
        edt: t.edt.filter(|_| reach(10.0)),
        t20: t.t20.filter(|_| reach(25.0)),
        t30: t.t30.filter(|_| reach(CLEAN_DECAY_DB)),
    };
    out.times = gated(edc_from_energy(&energy[start..end], sample_rate).times());

    let pre = start.saturating_sub((BAND_PREROLL_S * sample_rate) as usize);
    out.bands = OCTAVE_BANDS_HZ.map(|fc| {
        let bl = OctaveBandFilter::new(sample_rate, fc).filter(&left[pre..end]);
        let br = OctaveBandFilter::new(sample_rate, fc).filter(&right[pre..end]);
        let e: Vec<f64> = bl[start - pre..]
            .iter()
            .zip(&br[start - pre..])
            .map(|(a, b)| a * a + b * b)
            .collect();
        BandDecay {
            center_hz: fc,
            times: gated(edc_from_energy(&e, sample_rate).times()),
        }
    });
    out
}

/// Least-squares line `y = a + b·x`.
fn fit_line(pts: &[(f64, f64)]) -> Option<(f64, f64)> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for &(x, y) in pts {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
    }
    (sxx > 0.0).then(|| {
        let b = sxy / sxx;
        (my - b * mx, b)
    })
}
