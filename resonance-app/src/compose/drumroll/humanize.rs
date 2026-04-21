use resonance_audio::types::MidiNote;

/// Accent pattern applied on top of humanized velocities. Notes whose
/// start step matches the pattern get a velocity boost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccentPattern {
    None,
    /// Beats 1 and 3 of the bar (the "strong" beats in 4/4).
    Downbeats,
    /// Beats 2 and 4 (the classic snare accent).
    Backbeats,
    /// Every 4th step = once per beat at 16 steps/bar in 4/4.
    EveryBeat,
    /// Every 2nd step = eighth-note pulse at 16 steps/bar in 4/4.
    EveryEighth,
}

impl Default for AccentPattern {
    fn default() -> Self {
        Self::None
    }
}

impl AccentPattern {
    pub const ALL: [AccentPattern; 5] = [
        AccentPattern::None,
        AccentPattern::Downbeats,
        AccentPattern::Backbeats,
        AccentPattern::EveryBeat,
        AccentPattern::EveryEighth,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AccentPattern::None => "None",
            AccentPattern::Downbeats => "Downbeats (1, 3)",
            AccentPattern::Backbeats => "Backbeats (2, 4)",
            AccentPattern::EveryBeat => "Every beat",
            AccentPattern::EveryEighth => "Every eighth",
        }
    }

    /// Does the given step carry an accent under this pattern?
    /// `steps_per_beat` is `steps_per_bar / time_sig_num` (4 for 16ths in 4/4).
    fn is_accent(self, step_in_bar: u32, steps_per_beat: u32) -> bool {
        if steps_per_beat == 0 {
            return false;
        }
        match self {
            AccentPattern::None => false,
            AccentPattern::Downbeats => {
                // Beats 1, 3, 5, ... (even-indexed beats within the bar).
                let beat = step_in_bar / steps_per_beat;
                step_in_bar % steps_per_beat == 0 && beat % 2 == 0
            }
            AccentPattern::Backbeats => {
                let beat = step_in_bar / steps_per_beat;
                step_in_bar % steps_per_beat == 0 && beat % 2 == 1
            }
            AccentPattern::EveryBeat => step_in_bar % steps_per_beat == 0,
            AccentPattern::EveryEighth => {
                // Every half-beat at any resolution: every (steps_per_beat/2)
                // steps, falling back to every other step when the grid is
                // too coarse to split a beat.
                let half = (steps_per_beat / 2).max(1);
                step_in_bar % half == 0
            }
        }
    }
}

/// Where the humanizer applies its changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HumanizeScope {
    /// Only notes matching the currently selected pad's MIDI note.
    SelectedPad,
    /// Every note in the clip.
    AllPads,
}

impl Default for HumanizeScope {
    fn default() -> Self {
        Self::AllPads
    }
}

impl HumanizeScope {
    pub const ALL: [HumanizeScope; 2] = [HumanizeScope::SelectedPad, HumanizeScope::AllPads];

    pub fn as_str(self) -> &'static str {
        match self {
            HumanizeScope::SelectedPad => "Selected pad",
            HumanizeScope::AllPads => "All pads",
        }
    }
}

impl std::fmt::Display for AccentPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Display for HumanizeScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// All parameters needed to humanize a pattern. Values are in the 0..=1
/// sliders surfaced in the UI — the algorithm scales them to musically
/// sensible ranges internally.
#[derive(Debug, Clone)]
pub struct HumanizeParams {
    /// 0..=1. 1 means velocity can drift by up to ±0.5 around its current
    /// value.
    pub velocity_amount: f32,
    /// 0..=1. 1 means timing can drift by up to ±50% of a step.
    pub timing_amount: f32,
    /// 0..=1. 0 = straight, 0.5 ≈ triplet feel, 1 = hard dotted.
    pub swing: f32,
    pub accent_pattern: AccentPattern,
    /// 0..=1. How much velocity to add on an accent. 0.3 is a noticeable
    /// but not harsh lift.
    pub accent_amount: f32,
    /// Matches one of the pad MIDI notes when `scope == SelectedPad`.
    pub selected_pad_note: Option<u8>,
    pub scope: HumanizeScope,
    /// Step length in ticks (from the drumroll's `steps_per_bar`).
    pub step_ticks: u64,
    /// Steps per beat (`steps_per_bar / time_sig_num`).
    pub steps_per_beat: u32,
    /// Steps per bar — used to locate each hit's position within its bar.
    pub steps_per_bar: u32,
    /// Clip length in ticks — notes are clamped to stay inside.
    pub clip_length_ticks: u64,
    /// Random seed. Caller typically derives this from the system clock.
    pub seed: u64,
}

/// Apply humanization to a copy of `notes`, returning a new `Vec` ready
/// to replace them. Notes are first re-snapped to the nearest step so
/// repeated Apply clicks don't compound drift from previous runs.
pub fn humanize(notes: &[MidiNote], params: &HumanizeParams) -> Vec<MidiNote> {
    if params.step_ticks == 0 || params.clip_length_ticks == 0 {
        return notes.to_vec();
    }
    let mut rng = XorShift::new(params.seed);
    let mut out = Vec::with_capacity(notes.len());

    for note in notes {
        let in_scope = match params.scope {
            HumanizeScope::AllPads => true,
            HumanizeScope::SelectedPad => params.selected_pad_note == Some(note.note),
        };
        if !in_scope {
            out.push(note.clone());
            continue;
        }

        // Re-snap this note to the nearest step before applying jitter.
        let snapped = snap_to_step(note.start_tick, params.step_ticks);
        let step = snapped / params.step_ticks;
        let step_in_bar = if params.steps_per_bar == 0 {
            0
        } else {
            (step % params.steps_per_bar as u64) as u32
        };

        // Timing: ±amount * 50% of a step.
        let max_timing_shift =
            (params.timing_amount.clamp(0.0, 1.0) * 0.5 * params.step_ticks as f32) as i64;
        let timing_shift = if max_timing_shift == 0 {
            0
        } else {
            rng.symmetric_i64(max_timing_shift)
        };

        // Swing: systematic delay on odd steps within each beat. At 50%
        // swing the offbeat 16th sits at 2/3 of the beat (triplet feel).
        // Delay = swing * (1/3 of a step at 16ths/4/4); generalized we
        // scale it by (step_ticks / 3) so the feel is consistent at 8ths
        // and 32nds too.
        let swing_shift = if params.swing > 0.0 && params.steps_per_beat >= 2 {
            let step_within_beat = step_in_bar % params.steps_per_beat;
            if step_within_beat % 2 == 1 {
                (params.swing.clamp(0.0, 1.0) * (params.step_ticks as f32) / 3.0) as i64
            } else {
                0
            }
        } else {
            0
        };

        let mut new_start = snapped as i64 + timing_shift + swing_shift;
        // Clamp so the note stays inside the clip.
        new_start = new_start.clamp(0, params.clip_length_ticks.saturating_sub(1) as i64);

        // Velocity: ±amount * 0.5 around current value, then add accent.
        let mut new_vel = note.velocity;
        if params.velocity_amount > 0.0 {
            let range = params.velocity_amount.clamp(0.0, 1.0) * 0.5;
            new_vel += rng.symmetric_f32(range);
        }
        if params
            .accent_pattern
            .is_accent(step_in_bar, params.steps_per_beat)
        {
            new_vel += params.accent_amount.clamp(0.0, 1.0) * 0.3;
        }
        new_vel = new_vel.clamp(0.0, 1.0);

        out.push(MidiNote {
            note: note.note,
            velocity: new_vel,
            start_tick: new_start as u64,
            duration_ticks: note.duration_ticks,
        });
    }

    out
}

fn snap_to_step(tick: u64, step_ticks: u64) -> u64 {
    if step_ticks == 0 {
        return tick;
    }
    let q = tick / step_ticks;
    let rem = tick - q * step_ticks;
    if rem * 2 >= step_ticks {
        (q + 1) * step_ticks
    } else {
        q * step_ticks
    }
}

/// Minimal xorshift64 — deterministic given the same seed. Plenty
/// random-looking for humanization jitter; no crate dependency.
struct XorShift {
    state: u64,
}

impl XorShift {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Uniform in [0, 1).
    fn next_f32(&mut self) -> f32 {
        // Take 24 bits for a f32 mantissa — the top 24 are the most
        // uniform out of xorshift64.
        let bits = (self.next_u64() >> 40) as u32;
        (bits as f32) / ((1u32 << 24) as f32)
    }

    /// Uniform in [-range, range].
    fn symmetric_f32(&mut self, range: f32) -> f32 {
        (self.next_f32() * 2.0 - 1.0) * range
    }

    /// Uniform in [-range, range] for integer ticks.
    fn symmetric_i64(&mut self, range: i64) -> i64 {
        if range <= 0 {
            return 0;
        }
        let span = (range as u64).saturating_mul(2).saturating_add(1);
        (self.next_u64() % span) as i64 - range
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(n: u8, v: f32, t: u64) -> MidiNote {
        MidiNote {
            note: n,
            velocity: v,
            start_tick: t,
            duration_ticks: 120,
        }
    }

    fn default_params(seed: u64) -> HumanizeParams {
        HumanizeParams {
            velocity_amount: 0.0,
            timing_amount: 0.0,
            swing: 0.0,
            accent_pattern: AccentPattern::None,
            accent_amount: 0.0,
            selected_pad_note: None,
            scope: HumanizeScope::AllPads,
            step_ticks: 120,
            steps_per_beat: 4,
            steps_per_bar: 16,
            clip_length_ticks: 120 * 16,
            seed,
        }
    }

    #[test]
    fn zero_amounts_is_identity_after_snap() {
        let notes = vec![note(36, 0.9, 0), note(38, 0.9, 480)];
        let out = humanize(&notes, &default_params(42));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start_tick, 0);
        assert_eq!(out[1].start_tick, 480);
        assert_eq!(out[0].velocity, 0.9);
    }

    #[test]
    fn snap_is_applied_before_jitter() {
        // A note slightly off-grid at tick 5 should snap to 0 at step_ticks=120.
        let notes = vec![note(36, 0.9, 5)];
        let out = humanize(&notes, &default_params(1));
        assert_eq!(out[0].start_tick, 0);
    }

    #[test]
    fn selected_pad_scope_leaves_others_alone() {
        let notes = vec![note(36, 0.9, 0), note(38, 0.9, 120)];
        let mut p = default_params(7);
        p.velocity_amount = 1.0;
        p.scope = HumanizeScope::SelectedPad;
        p.selected_pad_note = Some(36);
        let out = humanize(&notes, &p);
        assert_eq!(out[1].velocity, 0.9);
        assert_eq!(out[1].start_tick, 120);
    }

    #[test]
    fn swing_delays_offbeats_only() {
        let notes = vec![
            note(36, 0.9, 0),   // step 0 — downbeat
            note(36, 0.9, 120), // step 1 — offbeat
            note(36, 0.9, 240), // step 2 — downbeat
            note(36, 0.9, 360), // step 3 — offbeat
        ];
        let mut p = default_params(3);
        p.swing = 1.0;
        let out = humanize(&notes, &p);
        assert_eq!(out[0].start_tick, 0);
        assert!(out[1].start_tick > 120);
        assert_eq!(out[2].start_tick, 240);
        assert!(out[3].start_tick > 360);
    }

    #[test]
    fn downbeat_accent_boosts_step_zero_only() {
        let notes = vec![
            note(36, 0.5, 0),        // beat 1 (downbeat)
            note(36, 0.5, 120 * 4),  // beat 2 (backbeat)
            note(36, 0.5, 120 * 8),  // beat 3 (downbeat)
            note(36, 0.5, 120 * 12), // beat 4 (backbeat)
        ];
        let mut p = default_params(2);
        p.accent_pattern = AccentPattern::Downbeats;
        p.accent_amount = 1.0;
        let out = humanize(&notes, &p);
        assert!(out[0].velocity > 0.5);
        assert_eq!(out[1].velocity, 0.5);
        assert!(out[2].velocity > 0.5);
        assert_eq!(out[3].velocity, 0.5);
    }

    #[test]
    fn backbeat_accent_hits_beats_2_and_4() {
        let notes = vec![
            note(38, 0.5, 0),
            note(38, 0.5, 120 * 4),
            note(38, 0.5, 120 * 8),
            note(38, 0.5, 120 * 12),
        ];
        let mut p = default_params(4);
        p.accent_pattern = AccentPattern::Backbeats;
        p.accent_amount = 1.0;
        let out = humanize(&notes, &p);
        assert_eq!(out[0].velocity, 0.5);
        assert!(out[1].velocity > 0.5);
        assert_eq!(out[2].velocity, 0.5);
        assert!(out[3].velocity > 0.5);
    }

    #[test]
    fn every_beat_accent_every_fourth_step() {
        let mut notes = Vec::new();
        for i in 0..16 {
            notes.push(note(42, 0.5, i * 120));
        }
        let mut p = default_params(5);
        p.accent_pattern = AccentPattern::EveryBeat;
        p.accent_amount = 1.0;
        let out = humanize(&notes, &p);
        for (i, n) in out.iter().enumerate() {
            if i % 4 == 0 {
                assert!(n.velocity > 0.5, "step {i} should be accented");
            } else {
                assert_eq!(n.velocity, 0.5, "step {i} should be untouched");
            }
        }
    }

    #[test]
    fn timing_jitter_stays_within_bounds() {
        let notes: Vec<MidiNote> = (0..16).map(|i| note(36, 0.9, i * 120)).collect();
        let mut p = default_params(99);
        p.timing_amount = 1.0;
        let out = humanize(&notes, &p);
        // Each note should lie within ±60 ticks (50% of step = 60) of
        // its snapped position, and inside the clip.
        for (i, n) in out.iter().enumerate() {
            let snapped = i as i64 * 120;
            let delta = n.start_tick as i64 - snapped;
            assert!(delta.abs() <= 60, "note {i} shift {delta} out of bounds");
            assert!((n.start_tick as u64) < p.clip_length_ticks);
        }
    }
}
