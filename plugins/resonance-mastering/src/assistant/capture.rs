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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_snapshot_is_chronological() {
        let mut c = CaptureBuffer::new(8, 48_000.0);
        let l = [1.0, 2.0, 3.0, 4.0];
        let r = [-1.0, -2.0, -3.0, -4.0];
        c.push(&l, &r);
        let (ls, rs) = c.snapshot_chrono();
        assert_eq!(ls, &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(rs, &[-1.0, -2.0, -3.0, -4.0]);
    }

    #[test]
    fn wrap_around_yields_most_recent() {
        let mut c = CaptureBuffer::new(4, 48_000.0);
        for i in 1..=10 {
            c.push(&[i as f32], &[-(i as f32)]);
        }
        // Should contain the last 4: 7, 8, 9, 10.
        let (ls, _) = c.snapshot_chrono();
        assert_eq!(ls, &[7.0, 8.0, 9.0, 10.0]);
    }

    #[test]
    fn clear_resets_everything() {
        let mut c = CaptureBuffer::new(4, 48_000.0);
        c.push(&[1.0, 2.0], &[3.0, 4.0]);
        c.clear();
        let (ls, rs) = c.snapshot_chrono();
        assert!(ls.is_empty());
        assert!(rs.is_empty());
    }
}
