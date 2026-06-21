//! Groove templates: extract a feel from played notes, apply a feel to
//! notes, plus a handful of stock templates.

use crate::types::{MidiNote, TempoMap};

use super::grid::{BarRuler, Division};

/// A per-step groove "feel": timing pushes and velocity accents measured
/// at each grid step within a bar.
#[derive(Debug, Clone, PartialEq)]
pub struct GrooveTemplate {
    /// Number of grid steps per bar (length of both vectors below).
    pub steps_per_bar: u32,
    /// Timing offset (ticks, signed) to add at each step. Positive =
    /// later / behind the beat.
    pub timing_offsets_ticks: Vec<i32>,
    /// Velocity multiplier at each step (1.0 = unchanged).
    pub velocity_scale: Vec<f32>,
}

impl GrooveTemplate {
    /// An identity template of the given step count (no timing/velocity change).
    pub fn identity(steps_per_bar: u32) -> Self {
        let n = steps_per_bar.max(1) as usize;
        GrooveTemplate {
            steps_per_bar: steps_per_bar.max(1),
            timing_offsets_ticks: vec![0; n],
            velocity_scale: vec![1.0; n],
        }
    }
}

/// Step index of `abs_tick` within its bar for grid step size `g`, plus
/// the tick position of that step (un-swung), via the bar ruler.
fn step_of(abs_tick: u64, ruler: &BarRuler, g: u64) -> (u64, u64) {
    let (bar_start, _bar_len) = ruler.bar_at(abs_tick);
    let local = abs_tick - bar_start;
    let k = (local as f64 / g as f64).round() as u64;
    (k, bar_start + k * g)
}

/// Measure a [`GrooveTemplate`] from `notes` quantized against `grid`.
///
/// For each grid step within a bar the template records the average
/// timing deviation (note start minus the nominal grid tick) and the
/// average velocity relative to the overall mean. Steps with no notes
/// get a zero offset and unit velocity scale. Note start ticks are
/// treated as absolute (i.e. relative to a bar-aligned origin).
pub fn extract_groove(notes: &[MidiNote], grid: Division, tempo: &TempoMap) -> GrooveTemplate {
    let ruler = BarRuler::new(tempo);
    let g = grid.ticks();
    let (_, bar_len) = ruler.bar_at(0);
    let steps_per_bar = (bar_len / g).max(1) as u32;
    let n = steps_per_bar as usize;

    let mut dev_sum = vec![0i64; n];
    let mut vel_sum = vec![0f64; n];
    let mut count = vec![0u32; n];
    let mut total_vel = 0f64;
    let mut total_notes = 0u32;

    for note in notes {
        let (k, grid_tick) = step_of(note.start_tick, &ruler, g);
        let step = (k % steps_per_bar as u64) as usize;
        dev_sum[step] += note.start_tick as i64 - grid_tick as i64;
        vel_sum[step] += note.velocity as f64;
        count[step] += 1;
        total_vel += note.velocity as f64;
        total_notes += 1;
    }

    let mean_vel = if total_notes > 0 {
        total_vel / total_notes as f64
    } else {
        0.0
    };

    let mut timing_offsets_ticks = vec![0i32; n];
    let mut velocity_scale = vec![1.0f32; n];
    for i in 0..n {
        if count[i] > 0 {
            timing_offsets_ticks[i] = (dev_sum[i] / count[i] as i64) as i32;
            if mean_vel > 0.0 {
                velocity_scale[i] = ((vel_sum[i] / count[i] as f64) / mean_vel) as f32;
            }
        }
    }

    GrooveTemplate {
        steps_per_bar,
        timing_offsets_ticks,
        velocity_scale,
    }
}

/// Apply a [`GrooveTemplate`] to the selected notes at the given strength
/// (`0.0` = no effect, `1.0` = full template).
///
/// Notes are processed by index and never reordered, merged, or dropped.
pub fn apply_groove(
    notes: &[MidiNote],
    selection: &[usize],
    template: &GrooveTemplate,
    strength: f32,
    tempo: &TempoMap,
) -> Vec<MidiNote> {
    let mut out = notes.to_vec();
    let spb = template.steps_per_bar;
    if spb == 0
        || template.timing_offsets_ticks.len() != spb as usize
        || template.velocity_scale.len() != spb as usize
    {
        return out; // malformed template → no-op
    }
    let strength = strength.clamp(0.0, 1.0) as f64;
    let ruler = BarRuler::new(tempo);

    for &i in selection {
        let Some(n) = out.get_mut(i) else { continue };
        let (bar_start, bar_len) = ruler.bar_at(n.start_tick);
        let g = (bar_len / spb as u64).max(1);
        let local = n.start_tick - bar_start;
        let k = (local as f64 / g as f64).round() as u64;
        let step = (k % spb as u64) as usize;

        let off = template.timing_offsets_ticks[step] as f64 * strength;
        n.start_tick = (n.start_tick as i64 + off.round() as i64).max(0) as u64;

        let scale = 1.0 + (template.velocity_scale[step] as f64 - 1.0) * strength;
        n.velocity = (n.velocity as f64 * scale).clamp(0.0, 1.0) as f32;
    }
    out
}

/// A small library of named stock grooves (16 sixteenth-note steps per
/// 4/4 bar).
pub fn stock_grooves() -> Vec<(String, GrooveTemplate)> {
    const STEPS: u32 = 16;
    // 16th-note grid in 4/4 → 120 ticks/step. A triplet-ish swing delay
    // of ~1/3 of a step lands the off-16ths on the swing feel.
    const SWING_DELAY: i32 = 40;

    // MPC-style 16th swing: delay every other 16th.
    let mut mpc = GrooveTemplate::identity(STEPS);
    for i in (1..STEPS as usize).step_by(2) {
        mpc.timing_offsets_ticks[i] = SWING_DELAY;
        mpc.velocity_scale[i] = 0.9;
    }
    // Accent the four down-beats slightly.
    for i in (0..STEPS as usize).step_by(4) {
        mpc.velocity_scale[i] = 1.05;
    }

    // Laid back: everything a touch behind the beat.
    let mut laid_back = GrooveTemplate::identity(STEPS);
    for o in laid_back.timing_offsets_ticks.iter_mut() {
        *o = 12;
    }

    // Pushed: everything a touch ahead of the beat.
    let mut pushed = GrooveTemplate::identity(STEPS);
    for o in pushed.timing_offsets_ticks.iter_mut() {
        *o = -12;
    }

    vec![
        ("MPC Swing".to_string(), mpc),
        ("Laid Back".to_string(), laid_back),
        ("Pushed".to_string(), pushed),
    ]
}
