//! Return EQ: a high-pass and a low-pass on the reverb's *input*, before
//! the pre-delay, the early reflections and the tank.
//!
//! This is the Abbey Road trick (warmth-width-depth.md §3.3): cut the
//! lows and the top out of what feeds the room, typically ~600 Hz and
//! ~10 kHz, so the reverb stops clouding the bass and hissing on
//! sibilants. Filtering the input rather than the output is the point:
//! the tank never builds up energy it would then have to throw away, and
//! the damping still shapes the tail on top.
//!
//! Each filter is a Butterworth: 12 dB/oct is one 2nd-order section
//! (Q 0.707); 18 dB/oct is a 2nd-order section of Q 1.0 plus a 1st-order
//! section at the same corner, the 3rd-order Butterworth factorisation.
//!
//! A filter that is off is not run at all, so with both off the input
//! reaches the tank untouched — bit-identical to the plugin before the
//! filters existed.

use resonance_dsp::Biquad;

/// Q of the 2nd-order section of a 3rd-order Butterworth.
const Q_THIRD_ORDER: f32 = 1.0;

/// One filter (HPF or LPF), both channels, up to two sections.
struct Section {
    on: bool,
    freq: f32,
    steep: bool,
    /// `[channel][stage]`; stage 1 runs only when `steep`.
    stages: [[Biquad; 2]; 2],
}

impl Section {
    fn new() -> Self {
        Self {
            on: false,
            freq: 0.0,
            steep: false,
            stages: [[Biquad::identity(); 2]; 2],
        }
    }

    fn configure(&mut self, on: bool, freq: f32, steep: bool, sr: f32, high_pass: bool) {
        if !on {
            self.on = false;
            return;
        }
        if !self.on || steep != self.steep {
            // Coming back on, or the section count changed: start from
            // clean state rather than whatever the stages held when they
            // last ran.
            for ch in &mut self.stages {
                for s in ch.iter_mut() {
                    s.reset();
                }
            }
        } else if (freq - self.freq).abs() < 1e-3 {
            return;
        }
        self.on = true;
        self.freq = freq;
        self.steep = steep;
        let q = if steep {
            Q_THIRD_ORDER
        } else {
            std::f32::consts::FRAC_1_SQRT_2
        };
        for ch in &mut self.stages {
            if high_pass {
                ch[0].set_high_pass(sr, freq, q);
                ch[1].set_first_order_high_pass(sr, freq);
            } else {
                ch[0].set_low_pass(sr, freq, q);
                ch[1].set_first_order_low_pass(sr, freq);
            }
        }
    }

    #[inline]
    fn process(&mut self, ch: usize, x: f32) -> f32 {
        let st = &mut self.stages[ch];
        let y = st[0].process(x);
        if self.steep {
            st[1].process(y)
        } else {
            y
        }
    }

    fn clear(&mut self) {
        for ch in &mut self.stages {
            for s in ch.iter_mut() {
                s.reset();
            }
        }
    }
}

/// The reverb's input high-pass and low-pass.
pub(super) struct ReturnEq {
    hpf: Section,
    lpf: Section,
}

impl ReturnEq {
    pub(super) fn new() -> Self {
        Self {
            hpf: Section::new(),
            lpf: Section::new(),
        }
    }

    /// Per-block configuration. Coefficients are only recomputed when a
    /// corner moved or a filter changed shape.
    pub(super) fn configure(
        &mut self,
        sr: f32,
        hpf_on: bool,
        hpf_hz: f32,
        lpf_on: bool,
        lpf_hz: f32,
        steep: bool,
    ) {
        self.hpf.configure(hpf_on, hpf_hz, steep, sr, true);
        self.lpf.configure(lpf_on, lpf_hz, steep, sr, false);
    }

    /// Filter one stereo sample. With both filters off this returns its
    /// input unchanged, without touching it.
    #[inline]
    pub(super) fn process(&mut self, mut l: f32, mut r: f32) -> (f32, f32) {
        if self.hpf.on {
            l = self.hpf.process(0, l);
            r = self.hpf.process(1, r);
        }
        if self.lpf.on {
            l = self.lpf.process(0, l);
            r = self.lpf.process(1, r);
        }
        (l, r)
    }

    pub(super) fn clear(&mut self) {
        self.hpf.clear();
        self.lpf.clear();
    }
}
