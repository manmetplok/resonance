/// One-pole DC-blocking high-pass: `y[n] = x[n] - x[n-1] + R*y[n-1]`.
///
/// The pole is derived from a cutoff in Hz and the sample rate,
/// `R = exp(-2π·fc/fs)`, so the corner stays put in Hz at any project
/// rate. A fixed `R` (the old `0.995`) put the corner at 38 Hz at 48 kHz
/// and 152 Hz at 192 kHz — audible bass loss that grew with the rate.
///
/// [`DcBlocker::DEFAULT_CUTOFF_HZ`] (5 Hz) is the choice for full-range
/// signal paths (amp output, mastering saturator): a first-order corner
/// at 5 Hz costs < 0.1 dB at 40 Hz and ~0.1 dB at a 7-string's 31 Hz low
/// B, while its ~32 ms time constant still settles a static waveshaper
/// or NAM bias well inside a beat. Call [`DcBlocker::set_cutoff`] from
/// the owner's `initialize` with the real sample rate.
#[derive(Clone, Copy)]
pub struct DcBlocker {
    x1: f32,
    y1: f32,
    r: f32,
}

impl Default for DcBlocker {
    /// [`DcBlocker::DEFAULT_CUTOFF_HZ`] at 48 kHz; owners re-derive the
    /// pole for the real rate via [`DcBlocker::set_cutoff`].
    fn default() -> Self {
        Self::new(Self::DEFAULT_CUTOFF_HZ, 48_000.0)
    }
}

impl DcBlocker {
    /// Corner for full-range signal paths (see the type docs).
    pub const DEFAULT_CUTOFF_HZ: f32 = 5.0;

    /// A blocker with its −3 dB corner at `cutoff_hz` for `sample_rate`.
    pub fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let mut b = Self {
            x1: 0.0,
            y1: 0.0,
            r: 0.0,
        };
        b.set_cutoff(cutoff_hz, sample_rate);
        b
    }

    /// Re-derive the pole for `cutoff_hz` at `sample_rate`. Keeps the
    /// filter state (call [`DcBlocker::reset`] too on a rate change).
    pub fn set_cutoff(&mut self, cutoff_hz: f32, sample_rate: f32) {
        let ratio = (cutoff_hz.max(0.0) / sample_rate.max(1.0)) as f64;
        self.r = (-std::f64::consts::TAU * ratio).exp() as f32;
    }

    /// The pole radius `R` currently in use.
    pub fn coefficient(&self) -> f32 {
        self.r
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}
