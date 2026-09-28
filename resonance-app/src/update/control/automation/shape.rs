//! `automation.shape` — generate a ramp / curve / LFO over a range
//! (automation-control-api.md §3 D3, §4.4, §4.6).
//!
//! The curve itself comes from the pure generator in
//! `resonance_common::automation_shape`; this handler is the glue around
//! it: resolve the target and the range, cut the range into per-bar tick
//! spans from the tempo / meter map, map `from` / `to` into the target's
//! real units, then turn the generated `(tick, real)` points into
//! `(sample, normalized)` breakpoints, splice them over the closed range
//! `[start, end]` of the current lane and commit ONE `SetLane`.

use super::edit::{check_call_limit, commit_points, current_lane, resolve_point_position};
use super::target::{resolve_write_target, ResolvedTarget};
use super::value::ValueDomain;
use crate::message::Message;
use crate::update::control::reply::{reject, success};
use crate::update::control::track::resolve_param_value;
use crate::Resonance;
use iced::Task;
use resonance_common::automation_shape::{
    generate_shape, replace_range, BarSpan, ShapeError, ShapeKind, ShapeRequest, StepQuantizer,
};
use resonance_common::{
    plugin_param_to_lane_value, real_to_lane_value, AutomationTarget, Breakpoint, GAIN_MAX_DB,
    GAIN_MIN_DB,
};
use resonance_control::methods::automation::{self as wire, AutomationValue, ShapeParams};
use resonance_control::methods::track::ParamValue;
use resonance_control::{Request, Response, RpcError};

pub(super) fn shape(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: ShapeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    match build(app, &params) {
        Ok((resolved, points, seed)) => match commit_points(app, &resolved, points, None) {
            Ok((mut result, task)) => {
                result.seed = seed;
                (success(request, &result), task)
            }
            Err(e) => reject(request, e),
        },
        Err(e) => reject(request, e),
    }
}

/// Everything short of the dispatch: the target, the lane's complete new
/// point list, and the `random_walk` seed to echo.
fn build(
    app: &Resonance,
    params: &ShapeParams,
) -> Result<(ResolvedTarget, Vec<Breakpoint>, Option<u64>), RpcError> {
    if params.cycles == Some(0) {
        return Err(RpcError::invalid_params("cycles must be at least 1"));
    }
    if params.resolution == Some(0) {
        return Err(RpcError::invalid_params(
            "resolution (points per bar) must be at least 1",
        ));
    }
    let resolved = resolve_write_target(app, &params.target)?;
    let start = resolve_point_position(app, &params.start)
        .map_err(|e| RpcError::new(e.kind(), format!("start: {}", e.message)))?;
    let end = resolve_point_position(app, &params.end)
        .map_err(|e| RpcError::new(e.kind(), format!("end: {}", e.message)))?;
    if end <= start {
        return Err(RpcError::invalid_params(format!(
            "end (sample {end}) must lie after start (sample {start}); end is where the \
             curve arrives at `to`, e.g. {{bar: 25}} for a sweep over bars 17-24"
        )));
    }

    let normalized = params.normalized;
    let from = real_input(&resolved, &params.from, normalized)
        .map_err(|e| RpcError::new(e.kind(), format!("from: {}", e.message)))?;
    let to = real_input(&resolved, &params.to, normalized)
        .map_err(|e| RpcError::new(e.kind(), format!("to: {}", e.message)))?;

    let grid = BarGrid::new(app, start, end);
    let (stepped, step_quantizer) = stepping(&resolved.domain, normalized);
    let request = ShapeRequest {
        shape: model_shape(params.shape),
        from,
        to,
        cycles: params.cycles,
        resolution: params.resolution,
        seed: params.seed,
        stepped,
        step_quantizer,
        logarithmic_unit: matches!(
            resolved.domain,
            ValueDomain::Gain | ValueDomain::Pan | ValueDomain::Mute
        ),
        bars: grid.spans.clone(),
    };
    let output = generate_shape(&request).map_err(shape_error)?;
    check_call_limit(output.points.len())?;

    let mut generated: Vec<Breakpoint> = Vec::with_capacity(output.points.len());
    for point in &output.points {
        let frame = grid.sample_at(app, point.tick);
        let value = lane_value(&resolved, point.value, normalized);
        let bp = Breakpoint::new(frame, value, point.curve);
        // Two ticks can only share a sample at an absurd density; the
        // later point (the end point last of all) wins.
        match generated.last_mut() {
            Some(last) if last.time_frames == frame => *last = bp,
            _ => generated.push(bp),
        }
    }

    let existing: &[Breakpoint] = current_lane(app, &resolved.target)
        .map(|lane| lane.points.as_slice())
        .unwrap_or(&[]);
    let points = replace_range(existing, start, end, generated, |p| p.time_frames);
    Ok((resolved, points, output.seed_used))
}

/// The range `[start, end]` cut into one tick span per bar it touches
/// (the first and last clipped to the range), plus what it takes to map a
/// generated tick back to a sample the way `resolve_position` does.
struct BarGrid {
    spans: Vec<BarSpan>,
    /// `(0-based bar, the bar's first tick)` for every span, in order.
    bars: Vec<(u32, u64)>,
    start: (u64, u64),
    end: (u64, u64),
}

impl BarGrid {
    fn new(app: &Resonance, start: u64, end: u64) -> Self {
        let map = &app.tempo_map;
        let sr = app.sample_rate;
        let (first_bar, _, _) = map.position_to_bars(start, sr);
        // 1-based -> 0-based.
        let first_bar = first_bar.saturating_sub(1);
        // Absolute ticks from bar 0: the exact bar grid, with no
        // sample -> tick truncation in it.
        let mut bar_tick: u64 = (0..first_bar).map(|b| map.bar_len_ticks_at(b)).sum();
        let snap = |tick: u64, grid: u64| if tick.abs_diff(grid) <= 1 { grid } else { tick };
        let start_tick = snap(map.sample_to_abs_tick(start, sr), bar_tick);

        let mut spans = Vec::new();
        let mut bars = Vec::new();
        let mut bar = first_bar;
        let mut end_tick = map.sample_to_abs_tick(end, sr).max(start_tick + 1);
        loop {
            let bar_ticks = map.bar_len_ticks_at(bar);
            let next_tick = bar_tick + bar_ticks;
            end_tick = snap(end_tick, next_tick);
            let lo = start_tick.max(bar_tick);
            let hi = end_tick.min(next_tick);
            if hi > lo {
                // A clipped first / last bar keeps its whole length, so
                // the generator gives it a proportional share of points.
                spans.push(BarSpan {
                    start_tick: lo,
                    end_tick: hi,
                    bar_ticks,
                });
                bars.push((bar, bar_tick));
            }
            if next_tick >= end_tick {
                break;
            }
            bar_tick = next_tick;
            bar += 1;
        }
        Self {
            spans,
            bars,
            start: (start_tick, start),
            end: (end_tick, end),
        }
    }

    /// The sample `tick` sits at: the range's own ends exactly, anything
    /// else as `bar start + tick offset` through the tempo map.
    fn sample_at(&self, app: &Resonance, tick: u64) -> u64 {
        if tick == self.start.0 {
            return self.start.1;
        }
        if tick == self.end.0 {
            return self.end.1;
        }
        let idx = self
            .bars
            .partition_point(|&(_, t)| t <= tick)
            .saturating_sub(1);
        let (bar, bar_tick) = self.bars[idx];
        let bar_sample = app.tempo_map.bar_to_sample(bar);
        app.tempo_map
            .tick_to_abs_sample(bar_sample, tick - bar_tick, app.sample_rate)
            .clamp(self.start.1, self.end.1)
    }
}

/// `from` / `to` as the number the generator interpolates: the target's
/// real value, or its `0..=1` lane value when the call is `normalized`.
/// Validation (ranges, labels, `"-inf"`) is [`ValueDomain::normalize`]'s,
/// so a bad value is refused with the words `set_lane` uses.
fn real_input(
    resolved: &ResolvedTarget,
    value: &AutomationValue,
    normalized: bool,
) -> Result<f64, RpcError> {
    let norm = resolved
        .domain
        .normalize(&resolved.target, value, normalized)?;
    if normalized {
        return Ok(f64::from(norm));
    }
    Ok(match &resolved.domain {
        // The floor (-60 dB or "-inf") is silence; interpolate from -60.
        ValueDomain::Gain if norm <= 0.0 => f64::from(GAIN_MIN_DB),
        ValueDomain::Gain | ValueDomain::Pan => number(value)
            .ok_or_else(|| RpcError::invalid_params(format!("expected a number, got {value:?}")))?,
        ValueDomain::Mute => {
            if norm >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        ValueDomain::Plugin(param) => {
            let requested = match value {
                AutomationValue::Number(n) => ParamValue::Number(*n),
                AutomationValue::Bool(b) => ParamValue::Number(if *b { 1.0 } else { 0.0 }),
                AutomationValue::Text(t) => ParamValue::Label(t.clone()),
            };
            resolve_param_value(param, &requested)?
        }
        // `normalize` already refused a non-normalized value here.
        ValueDomain::Normalized => f64::from(norm),
    })
}

fn number(value: &AutomationValue) -> Option<f64> {
    match value {
        AutomationValue::Number(n) => Some(*n),
        AutomationValue::Text(t) => t.trim().parse::<f64>().ok(),
        AutomationValue::Bool(_) => None,
    }
}

/// A generated value as the lane stores it, clamped into the target's
/// range first (a sine or a float's last bit may graze an edge).
fn lane_value(resolved: &ResolvedTarget, real: f64, normalized: bool) -> f32 {
    if normalized {
        return real.clamp(0.0, 1.0) as f32;
    }
    let target: &AutomationTarget = &resolved.target;
    match &resolved.domain {
        ValueDomain::Gain => real_to_lane_value(
            target,
            real.clamp(f64::from(GAIN_MIN_DB), f64::from(GAIN_MAX_DB)) as f32,
        ),
        ValueDomain::Pan => real_to_lane_value(target, real.clamp(-1.0, 1.0) as f32),
        ValueDomain::Mute => real_to_lane_value(target, if real >= 0.5 { 1.0 } else { 0.0 }),
        ValueDomain::Plugin(param) => {
            plugin_param_to_lane_value(real.clamp(param.min, param.max), param.min, param.max)
        }
        ValueDomain::Normalized => real.clamp(0.0, 1.0) as f32,
    }
}

/// Mute and stepped plugin parameters force a stepped curve and round
/// every value onto the target's own grid (§4.6).
fn stepping(domain: &ValueDomain, normalized: bool) -> (bool, Option<StepQuantizer>) {
    match domain {
        ValueDomain::Mute => (
            true,
            Some(StepQuantizer {
                min: 0.0,
                max: 1.0,
                steps: 2,
            }),
        ),
        ValueDomain::Plugin(param) if param.stepped => {
            let steps = ((param.max - param.min).round().max(0.0) as u32).saturating_add(1);
            let (min, max) = if normalized {
                (0.0, 1.0)
            } else {
                (param.min, param.max)
            };
            (true, Some(StepQuantizer { min, max, steps }))
        }
        _ => (false, None),
    }
}

fn model_shape(shape: wire::ShapeKind) -> ShapeKind {
    match shape {
        wire::ShapeKind::Ramp => ShapeKind::Ramp,
        wire::ShapeKind::Exp => ShapeKind::Exp,
        wire::ShapeKind::Sine => ShapeKind::Sine,
        wire::ShapeKind::Triangle => ShapeKind::Triangle,
        wire::ShapeKind::Square => ShapeKind::Square,
        wire::ShapeKind::Steps => ShapeKind::Steps,
        wire::ShapeKind::RandomWalk => ShapeKind::RandomWalk,
    }
}

/// The generator's refusals, worded for the wire.
fn shape_error(e: ShapeError) -> RpcError {
    match e {
        ShapeError::ExpRejectedLogarithmicUnit => RpcError::invalid_params(
            "exp is not available on volume, pan or mute: dB is already logarithmic; use \
             ramp",
        ),
        other => RpcError::invalid_params(other.to_string()),
    }
}
