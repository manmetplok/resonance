/// Simple delay line with power-of-2 buffer for cheap wrapping.
pub struct DelayLine {
    buffer: Vec<f32>,
    mask: usize,
    write_pos: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        let size = max_samples.max(2).next_power_of_two();
        let mut buffer = vec![0.0_f32; size];
        // `vec![0.0; size]` for large buffers is backed by lazy zero-fill
        // pages on Linux — the kernel only commits a physical page on
        // first access. If the audio thread is the first to touch them,
        // dozens of minor page faults fire under real-time pressure and
        // ALSA reports a BufferUnderrun on startup (most visible with
        // reverb, whose FDN/diffusion/pre-delay lines together cover
        // several hundred KB of cold pages). Touch one f32 per 4 KB page
        // now so the kernel commits everything before the audio thread
        // ever runs.
        const STRIDE: usize = 4096 / std::mem::size_of::<f32>();
        let ptr = buffer.as_mut_ptr();
        let mut i = 0;
        while i < size {
            // SAFETY: `i < size` and `ptr` came from a `Vec<f32>` of
            // length `size`. `write_volatile` prevents the compiler from
            // optimizing the store away on a buffer it knows is zero.
            unsafe { std::ptr::write_volatile(ptr.add(i), 0.0) };
            i += STRIDE;
        }
        Self {
            buffer,
            mask: size - 1,
            write_pos: 0,
        }
    }

    pub fn push(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) & self.mask;
    }

    pub fn tap(&self, delay: usize) -> f32 {
        let idx = self.write_pos.wrapping_sub(delay).wrapping_sub(1) & self.mask;
        self.buffer[idx]
    }

    /// Read with linear interpolation for fractional delays (modulation).
    pub fn tap_linear(&self, delay_frac: f32) -> f32 {
        let delay_int = delay_frac as usize;
        let frac = delay_frac - delay_int as f32;
        let a = self.tap(delay_int);
        let b = self.tap(delay_int + 1);
        a + frac * (b - a)
    }

    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }
}
