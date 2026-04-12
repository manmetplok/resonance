use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const MAX_ECHO_TAPS: usize = 8;

pub struct DelayViz {
    in_l_db: AtomicU32,
    in_r_db: AtomicU32,
    out_l_db: AtomicU32,
    out_r_db: AtomicU32,
    delay_time_ms: AtomicU32,
    current_bpm: AtomicU32,
    echo_times_l: [AtomicU32; MAX_ECHO_TAPS],
    echo_levels_l: [AtomicU32; MAX_ECHO_TAPS],
    echo_times_r: [AtomicU32; MAX_ECHO_TAPS],
    echo_levels_r: [AtomicU32; MAX_ECHO_TAPS],
}

impl DelayViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            in_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            delay_time_ms: AtomicU32::new(0.0f32.to_bits()),
            current_bpm: AtomicU32::new(0.0f32.to_bits()),
            echo_times_l: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            echo_levels_l: std::array::from_fn(|_| AtomicU32::new(f32::NEG_INFINITY.to_bits())),
            echo_times_r: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            echo_levels_r: std::array::from_fn(|_| AtomicU32::new(f32::NEG_INFINITY.to_bits())),
        })
    }

    pub fn store_peaks(&self, in_l_db: f32, in_r_db: f32, out_l_db: f32, out_r_db: f32) {
        self.in_l_db.store(in_l_db.to_bits(), Ordering::Relaxed);
        self.in_r_db.store(in_r_db.to_bits(), Ordering::Relaxed);
        self.out_l_db.store(out_l_db.to_bits(), Ordering::Relaxed);
        self.out_r_db.store(out_r_db.to_bits(), Ordering::Relaxed);
    }

    pub fn read_in_peaks_db(&self) -> (f32, f32) {
        (
            f32::from_bits(self.in_l_db.load(Ordering::Relaxed)),
            f32::from_bits(self.in_r_db.load(Ordering::Relaxed)),
        )
    }

    pub fn read_out_peaks_db(&self) -> (f32, f32) {
        (
            f32::from_bits(self.out_l_db.load(Ordering::Relaxed)),
            f32::from_bits(self.out_r_db.load(Ordering::Relaxed)),
        )
    }

    pub fn store_delay_time_ms(&self, ms: f32) {
        self.delay_time_ms.store(ms.to_bits(), Ordering::Relaxed);
    }

    pub fn read_delay_time_ms(&self) -> f32 {
        f32::from_bits(self.delay_time_ms.load(Ordering::Relaxed))
    }

    pub fn store_bpm(&self, bpm: f32) {
        self.current_bpm.store(bpm.to_bits(), Ordering::Relaxed);
    }

    pub fn read_bpm(&self) -> f32 {
        f32::from_bits(self.current_bpm.load(Ordering::Relaxed))
    }

    pub fn store_echo_taps(
        &self,
        times_l: &[f32; MAX_ECHO_TAPS],
        levels_l: &[f32; MAX_ECHO_TAPS],
        times_r: &[f32; MAX_ECHO_TAPS],
        levels_r: &[f32; MAX_ECHO_TAPS],
    ) {
        for i in 0..MAX_ECHO_TAPS {
            self.echo_times_l[i].store(times_l[i].to_bits(), Ordering::Relaxed);
            self.echo_levels_l[i].store(levels_l[i].to_bits(), Ordering::Relaxed);
            self.echo_times_r[i].store(times_r[i].to_bits(), Ordering::Relaxed);
            self.echo_levels_r[i].store(levels_r[i].to_bits(), Ordering::Relaxed);
        }
    }

    pub fn read_echo_taps(
        &self,
    ) -> (
        [f32; MAX_ECHO_TAPS],
        [f32; MAX_ECHO_TAPS],
        [f32; MAX_ECHO_TAPS],
        [f32; MAX_ECHO_TAPS],
    ) {
        let mut tl = [0.0f32; MAX_ECHO_TAPS];
        let mut ll = [0.0f32; MAX_ECHO_TAPS];
        let mut tr = [0.0f32; MAX_ECHO_TAPS];
        let mut lr = [0.0f32; MAX_ECHO_TAPS];
        for i in 0..MAX_ECHO_TAPS {
            tl[i] = f32::from_bits(self.echo_times_l[i].load(Ordering::Relaxed));
            ll[i] = f32::from_bits(self.echo_levels_l[i].load(Ordering::Relaxed));
            tr[i] = f32::from_bits(self.echo_times_r[i].load(Ordering::Relaxed));
            lr[i] = f32::from_bits(self.echo_levels_r[i].load(Ordering::Relaxed));
        }
        (tl, ll, tr, lr)
    }
}
