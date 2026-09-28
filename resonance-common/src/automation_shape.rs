//! Pure curve generator behind `automation.shape` (`automation-control-api.md`
//! §3 D3, §4.4, §4.6). This module has no knowledge of the control API, the
//! app or the engine — it only turns a shape request into a list of
//! `(tick, real_value, curve)` points, plus a pure helper to splice those
//! points into an existing sorted point list. The app's `automation.shape`
//! handler (`resonance-app/src/update/control/automation/shape.rs`) converts
//! ticks to sample frames and real values to normalized lane values.
//!
//! Positions are expressed in **ticks**, not sample frames: the caller
//! resolves bars/beats to ticks through the tempo/meter map (that logic lives
//! above this crate) and passes the result in as a list of per-bar
//! [`BarSpan`]s. Spacing points evenly within each bar's own tick span, rather
//! than evenly across the whole request, is what makes a 7/8 bar get the same
//! point count as a 4/4 bar (§4.6).
//!
//! Point *placement* is per bar, but point *values* follow each point's tick
//! position across the whole span, so a sweep progresses in time: a 7/8 bar
//! covers 7/8 as much of an `exp` sweep as a 4/4 bar does.
//!
//! Every shape's output ends with a point *at* the last bar's `end_tick`
//! holding `to` exactly (D3 in §3) — the value actually arrives, rather than
//! stopping one step short of it.

use std::fmt;

use crate::automation::CurveKind;

/// The per-call point-count limit from §4.4. `automation.shape` itself is
/// held to this; the ≤10,000-points-per-lane limit is enforced elsewhere
/// (this module only ever produces one call's worth of points).
pub const MAX_SHAPE_POINTS_PER_CALL: usize = 2048;

/// A shape kind from the §4.6 table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeKind {
    /// Two points: `from` at the start, `to` at the end. The engine
    /// interpolates the rest exactly, so no density is needed.
    Ramp,
    /// Geometric (exponential) interpolation from `from` to `to`. Requires
    /// both endpoints strictly positive; rejected outright for a
    /// logarithmic-unit target (§4.6: "dB is already logarithmic; use ramp").
    Exp,
    /// Oscillates between `from` and `to`, `cycles` times (default 1).
    Sine,
    /// Oscillates between `from` and `to` with straight-line legs and exact
    /// corners, `cycles` times.
    Triangle,
    /// Like [`ShapeKind::Triangle`] but stepped (holds each corner's value).
    Square,
    /// A rising (or falling) staircase from `from` to `to`, always stepped.
    Steps,
    /// A bounded random walk, deterministic from a seed.
    RandomWalk,
}

/// One bar's tick span, `[start_tick, end_tick)`. The caller supplies one of
/// these per bar covered by the shape request, in order, so a shape that
/// spans bars of different lengths (a 7/8 bar next to a 4/4 bar) still gets
/// the same point density in each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarSpan {
    pub start_tick: u64,
    pub end_tick: u64,
}

impl BarSpan {
    pub fn len_ticks(&self) -> u64 {
        self.end_tick.saturating_sub(self.start_tick)
    }
}

/// Quantizes a real value onto one of `steps` equally spaced values across
/// `min..=max` (inclusive at both ends). Used for mute (`steps: 2`, i.e. only
/// `min`/`max` survive) and stepped plugin params (`steps` = the param's
/// discrete value count).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepQuantizer {
    pub min: f64,
    pub max: f64,
    pub steps: u32,
}

impl StepQuantizer {
    /// Round `value` onto the nearest of this quantizer's discrete grid
    /// points. Degenerate ranges (`steps <= 1` or `max <= min`) collapse to
    /// `min`.
    pub fn quantize(&self, value: f64) -> f64 {
        if self.steps <= 1 || self.max <= self.min {
            return self.min;
        }
        let t = ((value - self.min) / (self.max - self.min)).clamp(0.0, 1.0);
        let n = (self.steps - 1) as f64;
        let idx = (t * n).round();
        self.min + (idx / n) * (self.max - self.min)
    }
}

/// Input to [`generate_shape`]. Every value is in the target's REAL units
/// (dB, pan −1..=1, the plugin's own min..=max, …) — this module never
/// normalizes, that is the caller's job once it knows the target.
#[derive(Debug, Clone)]
pub struct ShapeRequest {
    pub shape: ShapeKind,
    /// Real-unit value at the start of the span.
    pub from: f64,
    /// Real-unit value at the end of the span (the value the final point
    /// holds, per D3).
    pub to: f64,
    /// Oscillation count for `sine`/`triangle`/`square`. `None` = 1.
    pub cycles: Option<u32>,
    /// Points per bar for `exp`/`sine`/`steps`/`random_walk`. `None` = the
    /// shape's default from the §4.6 table. Ignored by `ramp`, `triangle`
    /// and `square` (their density comes from `cycles` instead).
    pub resolution: Option<u32>,
    /// Seed for `random_walk`'s xorshift generator. `None` = 0.
    pub seed: Option<u64>,
    /// Force every generated point to the `Stepped` curve and, if
    /// [`ShapeRequest::step_quantizer`] is set, round its value onto that
    /// quantizer's grid. Set this for `mute` and for a stepped plugin param
    /// — the target dictates it, not the shape.
    pub stepped: bool,
    /// Optional rounding grid applied when `stepped` is set.
    pub step_quantizer: Option<StepQuantizer>,
    /// The target's unit is already logarithmic (volume/pan/mute in §4.6) —
    /// `exp` is rejected for it ("dB is already logarithmic; use ramp").
    pub logarithmic_unit: bool,
    /// The span, one entry per bar, in order. Must be non-empty; each span's
    /// `end_tick` must be strictly greater than its `start_tick`.
    pub bars: Vec<BarSpan>,
}

/// One generated point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeneratedPoint {
    pub tick: u64,
    pub value: f64,
    pub curve: CurveKind,
}

/// Output of [`generate_shape`].
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeOutput {
    /// Generated points, sorted ascending by tick, ending with a point at
    /// the last bar's `end_tick` holding `to` (D3).
    pub points: Vec<GeneratedPoint>,
    /// The seed actually used, echoed back — only meaningful for
    /// `random_walk` (§4.6: "the seed used is echoed").
    pub seed_used: Option<u64>,
}

/// Why [`generate_shape`] refused a request.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeError {
    /// `exp` requires both `from` and `to` strictly positive.
    ExpRequiresPositive { from: f64, to: f64 },
    /// `exp` was requested for a target whose unit is already logarithmic.
    ExpRejectedLogarithmicUnit,
    /// `bars` was empty; there is no span to generate over.
    EmptyBarSpan,
    /// A bar span's `end_tick` was not strictly after its `start_tick`.
    InvalidBarSpan { start_tick: u64, end_tick: u64 },
    /// The request would produce more than [`MAX_SHAPE_POINTS_PER_CALL`]
    /// points. `max_value` is the largest value of `param_name` (`"resolution"`
    /// or `"cycles"`, depending on the shape) that would fit.
    TooManyPoints {
        count: usize,
        limit: usize,
        param_name: &'static str,
        max_value: u32,
    },
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShapeError::ExpRequiresPositive { from, to } => write!(
                f,
                "exp requires from and to to be > 0 (got from={from}, to={to})"
            ),
            ShapeError::ExpRejectedLogarithmicUnit => {
                write!(f, "dB is already logarithmic; use ramp")
            }
            ShapeError::EmptyBarSpan => write!(f, "shape needs at least one bar span"),
            ShapeError::InvalidBarSpan {
                start_tick,
                end_tick,
            } => write!(
                f,
                "bar span end_tick ({end_tick}) must be after start_tick ({start_tick})"
            ),
            ShapeError::TooManyPoints {
                count,
                limit,
                param_name,
                max_value,
            } => write!(
                f,
                "shape would generate {count} points, over the {limit}-point limit; \
                 use {param_name} <= {max_value} to fit"
            ),
        }
    }
}

impl std::error::Error for ShapeError {}

const DEFAULT_RESOLUTION_EXP_SINE: u32 = 16;
const DEFAULT_RESOLUTION_STEPS: u32 = 1;
const DEFAULT_RESOLUTION_RANDOM_WALK: u32 = 4;
const DEFAULT_CYCLES: u32 = 1;

fn default_resolution(shape: ShapeKind) -> u32 {
    match shape {
        ShapeKind::Exp | ShapeKind::Sine => DEFAULT_RESOLUTION_EXP_SINE,
        ShapeKind::Steps => DEFAULT_RESOLUTION_STEPS,
        ShapeKind::RandomWalk => DEFAULT_RESOLUTION_RANDOM_WALK,
        ShapeKind::Ramp | ShapeKind::Triangle | ShapeKind::Square => 0,
    }
}

/// The curve a shape uses absent `stepped` forcing it.
fn base_curve(shape: ShapeKind) -> CurveKind {
    match shape {
        ShapeKind::Steps | ShapeKind::Square => CurveKind::Stepped,
        ShapeKind::Ramp
        | ShapeKind::Exp
        | ShapeKind::Sine
        | ShapeKind::Triangle
        | ShapeKind::RandomWalk => CurveKind::Linear,
    }
}

/// A tiny xorshift64 PRNG. Not cryptographic, just deterministic and cheap —
/// exactly what a reproducible `random_walk` needs.
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        // xorshift's state must never be all-zero (it would stay zero
        // forever), so substitute a fixed nonzero constant for seed 0. The
        // *reported* seed (`seed_used`) is still the caller's original 0.
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in `[0.0, 1.0)`.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn validate_bars(bars: &[BarSpan]) -> Result<(), ShapeError> {
    if bars.is_empty() {
        return Err(ShapeError::EmptyBarSpan);
    }
    for bar in bars {
        if bar.end_tick <= bar.start_tick {
            return Err(ShapeError::InvalidBarSpan {
                start_tick: bar.start_tick,
                end_tick: bar.end_tick,
            });
        }
    }
    Ok(())
}

/// Evenly spaced grid ticks, `resolution` per bar, spaced within each bar's
/// own tick span (so a short bar and a long bar both get `resolution`
/// points). Point `k` of a bar sits at `start + k * len / resolution`, for
/// `k` in `0..resolution` — i.e. it marks the start of each of `resolution`
/// equal subdivisions, never the bar's own end (the caller adds the overall
/// end point separately, per D3).
fn grid_ticks(bars: &[BarSpan], resolution: u32) -> Vec<u64> {
    let mut ticks = Vec::with_capacity(bars.len() * resolution as usize);
    for bar in bars {
        let len = bar.len_ticks() as f64;
        for k in 0..resolution {
            let offset = (len * k as f64 / resolution as f64).round() as u64;
            ticks.push(bar.start_tick + offset);
        }
    }
    ticks
}

/// Ensure the last point sits exactly at `end_tick` holding `to` (D3):
/// override it in place if a point is already there, otherwise append one.
fn force_end_point(points: &mut Vec<(u64, f64)>, end_tick: u64, to: f64) {
    match points.last_mut() {
        Some(last) if last.0 == end_tick => last.1 = to,
        _ => points.push((end_tick, to)),
    }
}

/// Check the per-call point limit for a grid-based shape (`exp`/`sine`/
/// `steps`/`random_walk`), whose count is `resolution * num_bars + 1`.
fn check_grid_limit(count: usize, num_bars: usize) -> Result<(), ShapeError> {
    if count <= MAX_SHAPE_POINTS_PER_CALL {
        return Ok(());
    }
    let max_value = (MAX_SHAPE_POINTS_PER_CALL - 1)
        .checked_div(num_bars)
        .unwrap_or(0) as u32;
    Err(ShapeError::TooManyPoints {
        count,
        limit: MAX_SHAPE_POINTS_PER_CALL,
        param_name: "resolution",
        max_value,
    })
}

/// Check the per-call point limit for a cycle-based shape (`triangle`/
/// `square`), whose count is `2 * cycles + 1`.
fn check_cycle_limit(count: usize, cycles: u32) -> Result<(), ShapeError> {
    if count <= MAX_SHAPE_POINTS_PER_CALL {
        return Ok(());
    }
    let max_value = ((MAX_SHAPE_POINTS_PER_CALL - 1) / 2) as u32;
    let _ = cycles;
    Err(ShapeError::TooManyPoints {
        count,
        limit: MAX_SHAPE_POINTS_PER_CALL,
        param_name: "cycles",
        max_value,
    })
}

/// Generate the raw `(tick, value)` points for a shape, before the D3
/// end-point fixup and any `stepped` forcing. `resolution`/`cycles` are
/// already resolved (defaults applied) and count-checked by the caller.
fn raw_points(
    shape: ShapeKind,
    from: f64,
    to: f64,
    cycles: u32,
    resolution: u32,
    seed: u64,
    bars: &[BarSpan],
) -> Vec<(u64, f64)> {
    let start_tick = bars[0].start_tick;
    let end_tick = bars[bars.len() - 1].end_tick;
    // Values follow each point's position in *time* across the whole span,
    // not its index: a short 7/8 bar covers less of a sweep than a 4/4 bar,
    // even though both carry the same number of points. Ticks are the time
    // axis here; the tempo map is resolved above this crate.
    let span = (end_tick - start_tick) as f64;
    let frac = |tick: u64| (tick - start_tick) as f64 / span;

    match shape {
        ShapeKind::Ramp => vec![(start_tick, from), (end_tick, to)],

        ShapeKind::Exp => grid_ticks(bars, resolution)
            .into_iter()
            .map(|tick| (tick, from * (to / from).powf(frac(tick))))
            .collect(),

        ShapeKind::Sine => grid_ticks(bars, resolution)
            .into_iter()
            .map(|tick| {
                let phase = 2.0 * std::f64::consts::PI * cycles as f64 * frac(tick);
                let value = from + (to - from) * (1.0 - phase.cos()) / 2.0;
                (tick, value)
            })
            .collect(),

        ShapeKind::Steps => grid_ticks(bars, resolution)
            .into_iter()
            .map(|tick| (tick, from + (to - from) * frac(tick)))
            .collect(),

        ShapeKind::RandomWalk => {
            let ticks = grid_ticks(bars, resolution);
            let lo = from.min(to);
            let hi = from.max(to);
            let range = hi - lo;
            let n = ticks.len().max(1);
            // Step size scaled so the walk can plausibly cover the range
            // over the course of the grid, not just jitter near `from`.
            let step_scale = if range > 0.0 {
                range / n as f64 * 2.0
            } else {
                0.0
            };
            let mut rng = XorShift64::new(seed);
            let mut value = from.clamp(lo, hi);
            ticks
                .into_iter()
                .enumerate()
                .map(|(i, tick)| {
                    if i > 0 {
                        let r = rng.next_f64() * 2.0 - 1.0;
                        value = (value + r * step_scale).clamp(lo, hi);
                    }
                    (tick, value)
                })
                .collect()
        }

        ShapeKind::Triangle | ShapeKind::Square => {
            let corners = 2 * cycles;
            (0..=corners)
                .map(|k| {
                    let tick = start_tick + (span * k as f64 / corners as f64).round() as u64;
                    let value = if k % 2 == 0 { from } else { to };
                    (tick, value)
                })
                .collect()
        }
    }
}

/// Generate the points for one `automation.shape` call (§4.6). Values are
/// real-unit; the caller normalizes and converts ticks to sample frames for
/// the target it already resolved.
pub fn generate_shape(req: &ShapeRequest) -> Result<ShapeOutput, ShapeError> {
    validate_bars(&req.bars)?;

    if req.shape == ShapeKind::Exp {
        if req.logarithmic_unit {
            return Err(ShapeError::ExpRejectedLogarithmicUnit);
        }
        if !(req.from > 0.0 && req.to > 0.0) {
            return Err(ShapeError::ExpRequiresPositive {
                from: req.from,
                to: req.to,
            });
        }
    }

    let cycles = req.cycles.unwrap_or(DEFAULT_CYCLES).max(1);
    let resolution = req
        .resolution
        .unwrap_or_else(|| default_resolution(req.shape))
        .max(1);
    let seed = req.seed.unwrap_or(0);
    let num_bars = req.bars.len();

    match req.shape {
        ShapeKind::Exp | ShapeKind::Sine | ShapeKind::Steps | ShapeKind::RandomWalk => {
            check_grid_limit(resolution as usize * num_bars + 1, num_bars)?;
        }
        ShapeKind::Triangle | ShapeKind::Square => {
            check_cycle_limit(2 * cycles as usize + 1, cycles)?;
        }
        ShapeKind::Ramp => {}
    }

    let mut points = raw_points(
        req.shape, req.from, req.to, cycles, resolution, seed, &req.bars,
    );
    let end_tick = req.bars[req.bars.len() - 1].end_tick;
    force_end_point(&mut points, end_tick, req.to);

    let curve = if req.stepped {
        CurveKind::Stepped
    } else {
        base_curve(req.shape)
    };

    let generated = points
        .into_iter()
        .map(|(tick, value)| {
            let value = if req.stepped {
                req.step_quantizer
                    .map(|q| q.quantize(value))
                    .unwrap_or(value)
            } else {
                value
            };
            GeneratedPoint { tick, value, curve }
        })
        .collect();

    Ok(ShapeOutput {
        points: generated,
        seed_used: matches!(req.shape, ShapeKind::RandomWalk).then_some(seed),
    })
}

/// Splice generated points into an existing sorted point list: points whose
/// key falls in the closed `[start, end]` range are removed, the generated
/// points are added, and the result is re-sorted by key. Points outside the
/// range are kept untouched (§3 D3, §4.4 `automation.shape`'s "replaces
/// points in [start, end] and keeps the rest").
///
/// Generic over the point representation and its key so it works equally on
/// tick-keyed [`GeneratedPoint`]s and frame-keyed `Breakpoint`s once the
/// caller has converted units.
pub fn replace_range<P, F>(
    existing: &[P],
    start: u64,
    end: u64,
    generated: Vec<P>,
    key: F,
) -> Vec<P>
where
    P: Clone,
    F: Fn(&P) -> u64,
{
    let mut result: Vec<P> = existing
        .iter()
        .filter(|p| {
            let k = key(p);
            k < start || k > end
        })
        .cloned()
        .collect();
    result.extend(generated);
    result.sort_by_key(|p| key(p));
    result
}
