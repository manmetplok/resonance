//! Stereo ring buffer for assistant capture.
//!
//! The audio thread appends samples via `push`, wrapping when the ring
//! fills. The UI thread calls `snapshot_chrono` to read out the most-
//! recent N samples in chronological order for offline analysis.

pub struct CaptureBuffer {
    left: Vec<f32>,
    right: Vec<f32>,
    write_pos: usize,
    filled: usize,
    capacity: usize,
    sample_rate: f32,
}

impl CaptureBuffer {
    pub fn new(capacity: usize, sample_rate: f32) -> Self {
        Self {
            left: vec![0.0; capacity.max(1)],
            right: vec![0.0; capacity.max(1)],
            write_pos: 0,
            filled: 0,
            capacity: capacity.max(1),
            sample_rate,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn filled(&self) -> usize {
        self.filled
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
    }

    pub fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write_pos = 0;
        self.filled = 0;
    }

    /// Append a stereo block. Audio-thread hot path; avoids allocation.
    pub fn push(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        for i in 0..n {
            self.left[self.write_pos] = left[i];
            self.right[self.write_pos] = right[i];
            self.write_pos = (self.write_pos + 1) % self.capacity;
            self.filled = (self.filled + 1).min(self.capacity);
        }
    }

    /// Snapshot the contents in chronological order (oldest first).
    /// Allocates two Vecs — OK because this is called from the UI
    /// thread on demand, not the audio thread.
    pub fn snapshot_chrono(&self) -> (Vec<f32>, Vec<f32>) {
        let mut l = Vec::with_capacity(self.filled);
        let mut r = Vec::with_capacity(self.filled);
        if self.filled < self.capacity {
            l.extend_from_slice(&self.left[..self.filled]);
            r.extend_from_slice(&self.right[..self.filled]);
        } else {
            // Ring full: oldest is at write_pos, newest is write_pos-1.
            l.extend_from_slice(&self.left[self.write_pos..]);
            l.extend_from_slice(&self.left[..self.write_pos]);
            r.extend_from_slice(&self.right[self.write_pos..]);
            r.extend_from_slice(&self.right[..self.write_pos]);
        }
        (l, r)
    }
}

