//! Tempo-sync resolution for the delay time, following the
//! `resonance-delay` division-table convention.
//!
//! The conversion itself is pure arithmetic on the host tempo — no
//! transport snapshot, no plugin types, no egui — so both the audio
//! path ([`delay_seconds`]) and the editor (tick marks, the division
//! stepper readout, the drag-to-snap grid) run the *same* function and
//! are unit-testable without either. The editor used to fabricate a
//! `TempoInfo { time_sig_num: 4, time_sig_den: 4, playing: false, .. }`
//! in three separate drawing/input sites just to reach this table
//! (ba todo #1265).
//!
//! # Why no time signature
//!
//! The divisions are *note values*, not bars: `1/4` is a quarter note,
//! `1/8D` a dotted eighth, and [`DIVISION_BEATS`] measures each of them
//! in quarter-note beats — which is the unit the transport's `bpm` is
//! expressed in throughout this platform (see `resonance-audio`'s tempo
//! map, where a bar of 6/8 is 1440 ticks = 3 quarter notes at
//! `TICKS_PER_QUARTER_NOTE` = 480). A note value therefore lasts the
//! same wall-clock time in 4/4, 3/4 and 6/8, and feeding the meter into
//! this conversion would *introduce* an error rather than fix one. Meter
//! only matters for bar-relative quantities, and this table has none —
//! `1/1` here is a whole note, not "one bar".

use resonance_plugin::TempoInfo;

pub const DIVISION_LABELS: &[&str] = &[
    "1/1", "1/2", "1/2D", "1/2T", "1/4", "1/4D", "1/4T", "1/8", "1/8D", "1/8T", "1/16", "1/16T",
];

/// Length of each division in quarter-note beats.
const DIVISION_BEATS: &[f32] = &[
    4.0,       // 1/1
    2.0,       // 1/2
    3.0,       // 1/2D  (dotted)
    4.0 / 3.0, // 1/2T  (triplet)
    1.0,       // 1/4
    1.5,       // 1/4D
    2.0 / 3.0, // 1/4T
    0.5,       // 1/8
    0.75,      // 1/8D
    1.0 / 3.0, // 1/8T
    0.25,      // 1/16
    1.0 / 6.0, // 1/16T
];

/// Slowest tempo the grid is resolved at: below this a host tempo is
/// treated as 20 BPM, so a division can never blow the delay line up.
const MIN_BPM: f32 = 20.0;

/// Length of `division` in quarter-note beats, clamped to the table.
pub fn division_beats(division: usize) -> f32 {
    DIVISION_BEATS[division.min(DIVISION_BEATS.len() - 1)]
}

/// Length of `division` at `bpm`, in seconds. Unclamped: what the
/// buffer can actually deliver is [`delay_seconds`]' business.
pub fn division_seconds(bpm: f32, division: usize) -> f32 {
    let seconds_per_beat = 60.0 / bpm.max(MIN_BPM);
    seconds_per_beat * division_beats(division)
}

/// Length of `division` at `bpm`, in milliseconds — the editor's unit.
pub fn division_ms(bpm: f32, division: usize) -> f32 {
    division_seconds(bpm, division) * 1000.0
}

/// Grains per second when one grain is spawned per `division` at
/// `bpm` — the tempo-locked grain rate of `density_sync` (ba todo
/// #1322). Clamped to the declared density range so neither a crawling
/// nor a runaway host tempo can drive the cloud outside what the
/// Density knob itself can ask for.
pub fn density_hz(bpm: f32, division: usize) -> f32 {
    let seconds = division_seconds(bpm, division);
    (1.0 / seconds.max(f32::MIN_POSITIVE)).clamp(
        crate::params::DENSITY_MIN_HZ,
        crate::params::DENSITY_MAX_HZ,
    )
}

/// Resolve the grain rate for a block: the tempo-locked rate while
/// `sync` is on and the host reports a tempo, otherwise the
/// free-running `density_hz` knob. Re-resolved every block, so the
/// cloud re-locks the moment the tempo moves.
pub fn grain_density_hz(
    sync: bool,
    division: usize,
    free_hz: f32,
    tempo: Option<TempoInfo>,
) -> f32 {
    match (sync, tempo) {
        (true, Some(t)) => density_hz(t.bpm, division),
        _ => free_hz,
    }
}

/// Index of the division whose length is closest to `target_ms` at
/// `bpm` (the drag-to-snap grid). Ties resolve to the longer division,
/// i.e. the lower index; a target outside the table snaps to its
/// nearest end.
pub fn nearest_division(bpm: f32, target_ms: f32) -> usize {
    let mut best = 0;
    let mut best_err = f32::INFINITY;
    for division in 0..DIVISION_LABELS.len() {
        let err = (division_ms(bpm, division) - target_ms).abs();
        if err < best_err {
            best_err = err;
            best = division;
        }
    }
    best
}

/// Resolve sync/division/time/tempo into the nominal grain read position
/// in *seconds* behind the write head. Free-running (or sync without
/// host transport) falls back to `time_ms`.
pub fn delay_seconds(
    sync: bool,
    division: usize,
    time_ms: f32,
    tempo: Option<TempoInfo>,
    max_delay_seconds: f32,
) -> f32 {
    let raw = match (sync, tempo) {
        (true, Some(t)) => division_seconds(t.bpm, division),
        _ => time_ms * 0.001,
    };
    clamp_delay_seconds(raw, max_delay_seconds)
}

/// Clamp a delay length to what the delay line can deliver. The editor
/// readouts use it too, so they show what the DSP will actually play
/// rather than the nominal division.
pub fn clamp_delay_seconds(seconds: f32, max_delay_seconds: f32) -> f32 {
    seconds.clamp(0.001, max_delay_seconds)
}
