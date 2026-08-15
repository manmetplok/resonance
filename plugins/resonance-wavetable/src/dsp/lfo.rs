/// Multi-shape LFO: sine, triangle, saw, square, sample & hold.
use resonance_dsp::SimpleRng;

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum LfoShape {
    Sine = 0,
    Triangle = 1,
    Saw = 2,
    Square = 3,
    SampleAndHold = 4,
}

impl LfoShape {
    /// Display names, indexed by the `lfoN_shape` parameter's integer value.
    /// The editor's shape control reads this array instead of printing the
    /// bare integer.
    pub const LABELS: [&'static str; 5] = ["Sine", "Tri", "Saw", "Square", "S&H"];

    pub fn from_int(v: i32) -> Self {
        match v {
            0 => Self::Sine,
            1 => Self::Triangle,
            2 => Self::Saw,
            3 => Self::Square,
            4 => Self::SampleAndHold,
            _ => Self::Sine,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }
}

// ---------------------------------------------------------------------------
// Tempo sync
// ---------------------------------------------------------------------------

/// How an LFO gets its phase.
///
/// Three genuinely distinct states, resolved from the two parameters that
/// back them (`lfoN_sync` wins over `lfoN_retrigger`). The editor's segmented
/// control renders exactly these, so every segment maps to something the DSP
/// does — the dead "Env" segment and the "Sync"-that-meant-retrigger were ba
/// todo #1271.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LfoMode {
    /// One free-running phase shared by every voice, at `lfoN_rate` Hz.
    Free,
    /// Per-voice phase, reset on note-on, at `lfoN_rate` Hz.
    Retrig,
    /// One phase locked to the host transport at `lfoN_division`.
    Sync,
}

impl LfoMode {
    pub const LABELS: [&'static str; 3] = ["Free", "Retrig", "Sync"];

    pub fn from_params(sync: bool, retrigger: bool) -> Self {
        match (sync, retrigger) {
            (true, _) => Self::Sync,
            (false, true) => Self::Retrig,
            (false, false) => Self::Free,
        }
    }

    /// The `(sync, retrigger)` pair that produces this mode. Selecting a mode
    /// in the editor writes both, so the parameters can never hold a
    /// combination the control does not display.
    pub fn to_params(self) -> (bool, bool) {
        match self {
            Self::Free => (false, false),
            Self::Retrig => (false, true),
            Self::Sync => (true, false),
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }
}

/// Musical divisions a synced LFO can lock to, as the `lfoN_division`
/// parameter's integer value.
///
/// One full LFO cycle spans the named division. Straight, dotted (`D`) and
/// triplet (`T`) variants of each note value, plus multi-bar cycles for slow
/// evolving patches.
///
/// Cycle length is expressed in *quarter-note beats*, matching
/// `TempoInfo::song_pos_beats`. The bar-based entries scale with the host
/// time signature; the note-value entries do not, because "1/8" means an
/// eighth note in any meter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SyncDivision {
    EightBars = 0,
    FourBars = 1,
    TwoBars = 2,
    OneBar = 3,
    Half = 4,
    HalfTriplet = 5,
    QuarterDotted = 6,
    Quarter = 7,
    QuarterTriplet = 8,
    EighthDotted = 9,
    Eighth = 10,
    EighthTriplet = 11,
    SixteenthDotted = 12,
    Sixteenth = 13,
    SixteenthTriplet = 14,
    ThirtySecond = 15,
    SixtyFourth = 16,
}

impl SyncDivision {
    /// Display names, indexed by the parameter's integer value.
    ///
    /// These live here rather than in the editor so the mapping cannot drift.
    /// They should move onto the `IntParam` itself — and so reach host
    /// automation lanes and MCP — once `IntParam::with_value_to_string`
    /// exists (ba todo #1289, then #1292 for this crate). Until then an
    /// automation lane shows the raw index, which is the fleet-wide P9
    /// finding, not something this todo can fix locally.
    pub const LABELS: [&'static str; 17] = [
        "8 bars", "4 bars", "2 bars", "1 bar", "1/2", "1/2T", "1/4D", "1/4", "1/4T", "1/8D",
        "1/8", "1/8T", "1/16D", "1/16", "1/16T", "1/32", "1/64",
    ];

    /// The default: one cycle per beat.
    pub const DEFAULT: Self = Self::Quarter;

    pub fn from_int(v: i32) -> Self {
        match v {
            0 => Self::EightBars,
            1 => Self::FourBars,
            2 => Self::TwoBars,
            3 => Self::OneBar,
            4 => Self::Half,
            5 => Self::HalfTriplet,
            6 => Self::QuarterDotted,
            7 => Self::Quarter,
            8 => Self::QuarterTriplet,
            9 => Self::EighthDotted,
            10 => Self::Eighth,
            11 => Self::EighthTriplet,
            12 => Self::SixteenthDotted,
            13 => Self::Sixteenth,
            14 => Self::SixteenthTriplet,
            15 => Self::ThirtySecond,
            16 => Self::SixtyFourth,
            _ => Self::Quarter,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// Length of one LFO cycle in quarter-note beats.
    ///
    /// `beats_per_bar` comes from the host time signature (4.0 in 4/4) and is
    /// only used by the bar-based divisions.
    pub fn beats(self, beats_per_bar: f32) -> f32 {
        match self {
            Self::EightBars => 8.0 * beats_per_bar,
            Self::FourBars => 4.0 * beats_per_bar,
            Self::TwoBars => 2.0 * beats_per_bar,
            Self::OneBar => beats_per_bar,
            Self::Half => 2.0,
            Self::HalfTriplet => 4.0 / 3.0,
            Self::QuarterDotted => 1.5,
            Self::Quarter => 1.0,
            Self::QuarterTriplet => 2.0 / 3.0,
            Self::EighthDotted => 0.75,
            Self::Eighth => 0.5,
            Self::EighthTriplet => 1.0 / 3.0,
            Self::SixteenthDotted => 0.375,
            Self::Sixteenth => 0.25,
            Self::SixteenthTriplet => 1.0 / 6.0,
            Self::ThirtySecond => 0.125,
            Self::SixtyFourth => 0.0625,
        }
    }
}

/// BPM assumed when the host supplies no transport at all (offline renders,
/// the standalone editor harness). A synced LFO then free-runs at the tempo
/// it would have had, rather than stopping.
pub const FALLBACK_BPM: f32 = 120.0;

/// One block's transport facts, with every fallback already applied, so the
/// render path never has to think about a missing or malformed transport.
#[derive(Clone, Copy, Debug)]
pub struct TransportPlan {
    pub bpm: f32,
    pub beats_per_bar: f32,
    /// Song position in quarter-note beats, `Some` only while the transport
    /// is rolling. A synced LFO anchors its phase to this every block, which
    /// is what makes it survive a tempo change or a locate. When the
    /// transport is stopped it is `None` and the LFO free-runs at the synced
    /// rate, so the editor's animation keeps moving while auditioning.
    pub song_pos_beats: Option<f64>,
}

impl TransportPlan {
    pub fn resolve(tempo: Option<resonance_plugin::TempoInfo>) -> Self {
        match tempo {
            Some(t) => {
                let bpm = if t.bpm.is_finite() && t.bpm > 0.0 {
                    t.bpm
                } else {
                    FALLBACK_BPM
                };
                Self {
                    bpm,
                    beats_per_bar: beats_per_bar(t.time_sig_num, t.time_sig_den),
                    song_pos_beats: (t.playing && t.song_pos_beats.is_finite())
                        .then_some(t.song_pos_beats),
                }
            }
            None => Self {
                bpm: FALLBACK_BPM,
                beats_per_bar: 4.0,
                song_pos_beats: None,
            },
        }
    }

    /// This LFO's frequency in Hz for the block.
    pub fn lfo_rate_hz(&self, mode: LfoMode, division: SyncDivision, free_rate: f32) -> f32 {
        match mode {
            LfoMode::Sync => sync_rate_hz(self.bpm, division.beats(self.beats_per_bar)),
            LfoMode::Free | LfoMode::Retrig => free_rate,
        }
    }

    /// The phase a synced LFO should be at right now, or `None` when there is
    /// nothing to anchor to (not synced, or transport stopped).
    pub fn lfo_anchor_phase(&self, mode: LfoMode, division: SyncDivision) -> Option<f32> {
        if mode != LfoMode::Sync {
            return None;
        }
        self.song_pos_beats
            .map(|pos| sync_phase(pos, division.beats(self.beats_per_bar)))
    }
}

/// Beats per bar for a host time signature, in quarter notes.
pub fn beats_per_bar(time_sig_num: u16, time_sig_den: u16) -> f32 {
    if time_sig_num == 0 || time_sig_den == 0 {
        return 4.0;
    }
    time_sig_num as f32 * 4.0 / time_sig_den as f32
}

/// LFO frequency for a tempo-synced cycle.
///
/// `bpm` is quarter notes per minute and `cycle_beats` is the cycle length
/// from [`SyncDivision::beats`], so one cycle takes `cycle_beats` beats at
/// `bpm` — 120 BPM at 1/4 is 2 Hz, at 1/8 is 4 Hz, at 1 bar in 4/4 is 0.5 Hz.
pub fn sync_rate_hz(bpm: f32, cycle_beats: f32) -> f32 {
    if cycle_beats <= 0.0 || !bpm.is_finite() || bpm <= 0.0 {
        return 0.0;
    }
    (bpm / 60.0) / cycle_beats
}

/// Absolute LFO phase (0..1) for a song position, so a synced LFO lands on
/// the same point of its cycle at the same musical position no matter how
/// the transport got there.
///
/// This is what makes sync survive a tempo change or a locate: the phase is
/// derived from the timeline, not integrated from wherever the LFO happened
/// to be.
pub fn sync_phase(song_pos_beats: f64, cycle_beats: f32) -> f32 {
    if cycle_beats <= 0.0 || !song_pos_beats.is_finite() {
        return 0.0;
    }
    let cycles = song_pos_beats / cycle_beats as f64;
    (cycles - cycles.floor()) as f32
}

#[derive(Clone)]
pub struct MultiLfo {
    pub phase: f32,
    phase_inc: f32,
    prev_phase: f32,
    sh_value: f32,
}

impl MultiLfo {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            phase_inc: 0.0,
            prev_phase: 0.0,
            sh_value: 0.0,
        }
    }

    pub fn set_rate(&mut self, rate_hz: f32, sample_rate: f32) {
        self.phase_inc = rate_hz / sample_rate;
    }

    pub fn reset_phase(&mut self) {
        self.phase = 0.0;
        self.prev_phase = 0.0;
    }

    /// Jump the phase to an absolute position, without treating the jump as
    /// a cycle wrap.
    ///
    /// Used once per block by a tempo-synced LFO to re-anchor on the host's
    /// song position. `prev_phase` follows the new phase so the S&H latch
    /// (which fires on `phase < prev_phase`) does not mistake a locate for a
    /// wrap and re-roll on every block.
    pub fn set_phase(&mut self, phase: f32) {
        let p = if phase.is_finite() {
            phase - phase.floor()
        } else {
            0.0
        };
        self.phase = p;
        self.prev_phase = p;
    }

    /// Value at the current phase, in -1..1, **without** advancing.
    ///
    /// Split out from [`Self::advance`] because the two run at different
    /// rates on the audio path: the phase has to move every sample to stay
    /// continuous, but the value is only consumed by the modulation matrix,
    /// which is evaluated at control rate. For the default sine shape this
    /// is a `sin()` call — at 32 voices × 3 LFOs × 48 kHz, keeping it off
    /// the per-sample path is worth several million transcendental calls a
    /// second.
    #[inline]
    pub fn value(&self, shape: LfoShape) -> f32 {
        match shape {
            LfoShape::Sine => (self.phase * std::f32::consts::TAU).sin(),
            LfoShape::Triangle => {
                if self.phase < 0.25 {
                    self.phase * 4.0
                } else if self.phase < 0.75 {
                    2.0 - self.phase * 4.0
                } else {
                    self.phase * 4.0 - 4.0
                }
            }
            LfoShape::Saw => 2.0 * self.phase - 1.0,
            LfoShape::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleAndHold => self.sh_value,
        }
    }

    /// Advance the phase by one sample.
    ///
    /// Must be called once per sample regardless of whether [`Self::value`]
    /// was read, so LFO phase stays sample-accurate and the S&H latch fires
    /// on the exact wrap sample.
    #[inline]
    pub fn advance(&mut self, shape: LfoShape, rng: &mut SimpleRng) {
        self.prev_phase = self.phase;
        self.phase += self.phase_inc;
        self.phase -= self.phase.floor();

        // Latch a new S&H value when the phase wrapped on this advance.
        // Done after the value read so the value held for *this* sample
        // matches what the user saw the previous frame, and the new
        // random value is what subsequent samples in this cycle hear.
        // Pulling the RNG out of the pre-advance match avoids calling
        // it for every other LFO shape (the old code ran the RNG
        // unconditionally inside the SH branch even when the phase
        // hadn't wrapped — multiplied across 32 voices × 3 LFOs that
        // was a few million unused RNG calls per second).
        if matches!(shape, LfoShape::SampleAndHold) && self.phase < self.prev_phase {
            self.sh_value = (rng.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
    }

    /// Read the current value, then advance one sample.
    ///
    /// Exactly `value()` followed by `advance()`; kept for callers that need
    /// both every sample.
    #[inline]
    pub fn next(&mut self, shape: LfoShape, rng: &mut SimpleRng) -> f32 {
        let out = self.value(shape);
        self.advance(shape, rng);
        out
    }
}

impl Default for MultiLfo {
    fn default() -> Self {
        Self::new()
    }
}
