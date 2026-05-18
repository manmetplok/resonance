//! Dilated-conv state ring buffer.
//!
//! Stores the last `capacity` channel-vectors written; `capacity` is rounded
//! up to a power of two so the wraparound is a bitmask instead of a modulo.

pub(super) struct RingBuffer {
    data: Vec<f32>,
    mask: usize, // capacity - 1 (power-of-2 bitmask)
    channels: usize,
    write_pos: usize,
}

impl RingBuffer {
    pub(super) fn new(min_capacity: usize, channels: usize) -> Self {
        // Round up to next power of 2 so we can use bitmask instead of modulo
        let capacity = min_capacity.next_power_of_two();
        Self {
            data: vec![0.0; capacity * channels],
            mask: capacity - 1,
            channels,
            write_pos: 0,
        }
    }

    #[inline(always)]
    pub(super) fn write(&mut self, values: &[f32]) {
        let base = self.write_pos * self.channels;
        self.data[base..base + self.channels].copy_from_slice(&values[..self.channels]);
        self.write_pos = (self.write_pos + 1) & self.mask;
    }

    #[inline(always)]
    pub(super) fn read_delayed(&self, delay: usize) -> &[f32] {
        let pos = (self.write_pos.wrapping_add(self.mask).wrapping_sub(delay)) & self.mask;
        let base = pos * self.channels;
        &self.data[base..base + self.channels]
    }

    #[inline(always)]
    pub(super) fn read_current(&self) -> &[f32] {
        self.read_delayed(0)
    }

    pub(super) fn reset(&mut self) {
        self.data.fill(0.0);
        self.write_pos = 0;
    }
}
