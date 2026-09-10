use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_dsp::linear_to_db;
use resonance_metering::{AtomicF32, AtomicF32Array, AtomicF32Pair};

pub const MAX_ECHO_TAPS: usize = 8;

/// Echo taps for the two channels: time in ms and level in dB per tap.
/// A tap that this channel does not carry (ping-pong alternates) is
/// parked at time `0.0` / level `-inf`, which the editor skips.
pub struct EchoTaps {
    pub times_l: [f32; MAX_ECHO_TAPS],
    pub levels_l: [f32; MAX_ECHO_TAPS],
    pub times_r: [f32; MAX_ECHO_TAPS],
    pub levels_r: [f32; MAX_ECHO_TAPS],
}

impl EchoTaps {
    fn silent() -> Self {
        Self {
            times_l: [0.0; MAX_ECHO_TAPS],
            levels_l: [f32::NEG_INFINITY; MAX_ECHO_TAPS],
            times_r: [0.0; MAX_ECHO_TAPS],
            levels_r: [f32::NEG_INFINITY; MAX_ECHO_TAPS],
        }
    }
}

/// Where the repeats land, per channel, for the current delay times and
/// route (ba todo #1331). The left and right trains used to be the same
/// array, so ping-pong and any stereo offset were invisible in the
/// editor no matter what the DSP was doing.
///
/// * stereo / dual (`routing != 1`) — two independent trains, each
///   repeating at its own delay time; a stereo offset pulls the right
///   train away from the left.
/// * ping-pong (`routing == 1`) — one train that alternates channels,
///   so tap n sits at the *cumulative* time of the bounces before it
///   (`d_l`, `d_l + d_r`, `2·d_l + d_r`, …) and only one channel draws
///   each tap. With an offset the bounce is uneven, which is exactly
///   what the picture should show.
///
/// Levels decay geometrically with the feedback (`fb^n`), accumulated
/// multiplicatively rather than via `powf` per tap.
pub fn echo_taps(delay_l_ms: f32, delay_r_ms: f32, feedback: f32, routing: i32) -> EchoTaps {
    let mut taps = EchoTaps::silent();
    let fb = feedback.clamp(0.0, 1.0);
    let mut fb_gain = 1.0f32;

    if routing == 1 {
        let mut t = 0.0f32;
        for tap in 0..MAX_ECHO_TAPS {
            // Bounces alternate L, R, L, … starting on the channel the
            // mono input is written to.
            let on_left = tap % 2 == 0;
            t += if on_left { delay_l_ms } else { delay_r_ms };
            fb_gain *= fb;
            let level = linear_to_db(fb_gain);
            if on_left {
                taps.times_l[tap] = t;
                taps.levels_l[tap] = level;
            } else {
                taps.times_r[tap] = t;
                taps.levels_r[tap] = level;
            }
        }
    } else {
        for tap in 0..MAX_ECHO_TAPS {
            let n = (tap + 1) as f32;
            fb_gain *= fb;
            let level = linear_to_db(fb_gain);
            taps.times_l[tap] = delay_l_ms * n;
            taps.levels_l[tap] = level;
            taps.times_r[tap] = delay_r_ms * n;
            taps.levels_r[tap] = level;
        }
    }
    taps
}

pub struct DelayViz {
    in_db: AtomicF32Pair,
    out_db: AtomicF32Pair,
    delay_time_ms: AtomicF32,
    current_bpm: AtomicF32,
    echo_times_l: AtomicF32Array<MAX_ECHO_TAPS>,
    echo_levels_l: AtomicF32Array<MAX_ECHO_TAPS>,
    echo_times_r: AtomicF32Array<MAX_ECHO_TAPS>,
    echo_levels_r: AtomicF32Array<MAX_ECHO_TAPS>,
}

impl DelayViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_db: AtomicF32Pair::new(f32::NEG_INFINITY),
            out_db: AtomicF32Pair::new(f32::NEG_INFINITY),
            delay_time_ms: AtomicF32::new(0.0),
            current_bpm: AtomicF32::new(0.0),
            echo_times_l: AtomicF32Array::new(0.0),
            echo_levels_l: AtomicF32Array::new(f32::NEG_INFINITY),
            echo_times_r: AtomicF32Array::new(0.0),
            echo_levels_r: AtomicF32Array::new(f32::NEG_INFINITY),
        })
    }

    pub fn store_peaks(&self, in_l_db: f32, in_r_db: f32, out_l_db: f32, out_r_db: f32) {
        self.in_db.store(in_l_db, in_r_db);
        self.out_db.store(out_l_db, out_r_db);
    }

    pub fn read_in_peaks_db(&self) -> (f32, f32) {
        self.in_db.load()
    }

    pub fn read_out_peaks_db(&self) -> (f32, f32) {
        self.out_db.load()
    }

    pub fn store_delay_time_ms(&self, ms: f32) {
        self.delay_time_ms.store(ms, Ordering::Relaxed);
    }

    pub fn read_delay_time_ms(&self) -> f32 {
        self.delay_time_ms.load(Ordering::Relaxed)
    }

    pub fn store_bpm(&self, bpm: f32) {
        self.current_bpm.store(bpm, Ordering::Relaxed);
    }

    pub fn read_bpm(&self) -> f32 {
        self.current_bpm.load(Ordering::Relaxed)
    }

    pub fn store_taps(&self, taps: &EchoTaps) {
        self.store_echo_taps(
            &taps.times_l,
            &taps.levels_l,
            &taps.times_r,
            &taps.levels_r,
        );
    }

    pub fn store_echo_taps(
        &self,
        times_l: &[f32; MAX_ECHO_TAPS],
        levels_l: &[f32; MAX_ECHO_TAPS],
        times_r: &[f32; MAX_ECHO_TAPS],
        levels_r: &[f32; MAX_ECHO_TAPS],
    ) {
        self.echo_times_l.store(times_l);
        self.echo_levels_l.store(levels_l);
        self.echo_times_r.store(times_r);
        self.echo_levels_r.store(levels_r);
    }

    pub fn read_echo_taps(
        &self,
    ) -> (
        [f32; MAX_ECHO_TAPS],
        [f32; MAX_ECHO_TAPS],
        [f32; MAX_ECHO_TAPS],
        [f32; MAX_ECHO_TAPS],
    ) {
        (
            self.echo_times_l.load(),
            self.echo_levels_l.load(),
            self.echo_times_r.load(),
            self.echo_levels_r.load(),
        )
    }
}
