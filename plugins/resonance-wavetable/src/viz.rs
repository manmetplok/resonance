//! Lock-free viz state shared between the audio thread (writer) and the
//! editor thread (reader).
//!
//! Scalars are stored as `AtomicU32` containing the `f32::to_bits()` of the
//! value. The scope buffer is double-buffered via a seq-lock so the reader
//! can get a tear-free snapshot without blocking the writer.

use std::sync::atomic::{AtomicU32, Ordering};

/// Number of stereo frames in the oscilloscope ring.
pub const SCOPE_FRAMES: usize = 256;
/// Scope buffer length in f32 values (stereo interleaved: L,R,L,R,...).
pub const SCOPE_LEN: usize = SCOPE_FRAMES * 2;

/// Lock-free per-plugin viz state.
///
/// One `Arc<WavetableVizState>` is shared between the audio thread
/// (`SynthEngine::process`) and the editor thread (via the plugin editor).
pub struct WavetableVizState {
    /// Current LFO phase in 0..1, one per LFO (global or representative voice).
    pub lfo_phases: [AtomicU32; 3],
    /// Current amp envelope value 0..1 for the representative voice.
    pub env_amp_value: AtomicU32,
    /// Current mod envelope value 0..1 for the representative voice.
    pub env_mod_value: AtomicU32,
    /// Current amp envelope stage (0=Idle, 1=Attack, 2=Decay, 3=Sustain, 4=Release).
    pub env_amp_stage: AtomicU32,
    /// Current post-modulation filter cutoff in Hz.
    pub filter_cutoff_live: AtomicU32,
    /// Current post-modulation osc1 position 0..1.
    pub osc1_position_live: AtomicU32,
    /// Current post-modulation osc2 position 0..1.
    pub osc2_position_live: AtomicU32,
    /// Number of active voices.
    pub active_voice_count: AtomicU32,
    /// Sequence counter for the scope double-buffer (seq-lock).
    /// Even = stable, odd = mid-update.
    pub scope_seq: AtomicU32,
    /// Published scope buffer readable by the UI thread.
    pub scope_front: [AtomicU32; SCOPE_LEN],
    /// Monotonic total samples pushed into the scope (for rate display).
    pub scope_sample_count: AtomicU32,
}

impl WavetableVizState {
    pub fn new() -> Self {
        Self {
            lfo_phases: [const { AtomicU32::new(0) }; 3],
            env_amp_value: AtomicU32::new(0),
            env_mod_value: AtomicU32::new(0),
            env_amp_stage: AtomicU32::new(0),
            filter_cutoff_live: AtomicU32::new(8000f32.to_bits()),
            osc1_position_live: AtomicU32::new(0),
            osc2_position_live: AtomicU32::new(0),
            active_voice_count: AtomicU32::new(0),
            scope_seq: AtomicU32::new(0),
            scope_front: [const { AtomicU32::new(0) }; SCOPE_LEN],
            scope_sample_count: AtomicU32::new(0),
        }
    }

    // -- scalar writers (audio thread) -------------------------------------

    #[inline]
    pub fn store_lfo_phase(&self, idx: usize, phase: f32) {
        self.lfo_phases[idx].store(phase.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn store_env_amp(&self, value: f32, stage: u32) {
        self.env_amp_value.store(value.to_bits(), Ordering::Relaxed);
        self.env_amp_stage.store(stage, Ordering::Relaxed);
    }

    #[inline]
    pub fn store_env_mod(&self, value: f32) {
        self.env_mod_value.store(value.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn store_filter_cutoff_live(&self, hz: f32) {
        self.filter_cutoff_live.store(hz.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn store_osc_positions(&self, osc1: f32, osc2: f32) {
        self.osc1_position_live.store(osc1.to_bits(), Ordering::Relaxed);
        self.osc2_position_live.store(osc2.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn store_active_voice_count(&self, n: u32) {
        self.active_voice_count.store(n, Ordering::Relaxed);
    }

    /// Publish a full scope frame from interleaved stereo samples.
    ///
    /// Audio thread: buffers samples locally into a small ring and calls
    /// this at block end to push the most recent `SCOPE_LEN` values.
    pub fn publish_scope(&self, samples: &[f32; SCOPE_LEN], added_frames: u32) {
        // Seq-lock: bump to odd (in progress), write, bump to even (committed).
        self.scope_seq.fetch_add(1, Ordering::Release);
        for (dst, src) in self.scope_front.iter().zip(samples.iter()) {
            dst.store(src.to_bits(), Ordering::Relaxed);
        }
        self.scope_seq.fetch_add(1, Ordering::Release);
        self.scope_sample_count
            .fetch_add(added_frames, Ordering::Relaxed);
    }

    // -- reader (editor thread) --------------------------------------------

    /// Read a tear-free snapshot of the viz state.
    #[allow(dead_code)] // consumed by the editor in Phase 3
    pub fn read_snapshot(&self) -> VizSnapshot {
        // Seq-lock loop for the scope buffer: retry if the writer changed
        // the counter mid-read.
        let mut scope_samples = [0.0f32; SCOPE_LEN];
        loop {
            let seq_before = self.scope_seq.load(Ordering::Acquire);
            if seq_before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            for (dst, src) in scope_samples.iter_mut().zip(self.scope_front.iter()) {
                *dst = f32::from_bits(src.load(Ordering::Relaxed));
            }
            let seq_after = self.scope_seq.load(Ordering::Acquire);
            if seq_before == seq_after {
                break;
            }
            std::hint::spin_loop();
        }

        VizSnapshot {
            lfo_phases: [
                f32::from_bits(self.lfo_phases[0].load(Ordering::Relaxed)),
                f32::from_bits(self.lfo_phases[1].load(Ordering::Relaxed)),
                f32::from_bits(self.lfo_phases[2].load(Ordering::Relaxed)),
            ],
            env_amp_value: f32::from_bits(self.env_amp_value.load(Ordering::Relaxed)),
            env_amp_stage: self.env_amp_stage.load(Ordering::Relaxed),
            env_mod_value: f32::from_bits(self.env_mod_value.load(Ordering::Relaxed)),
            filter_cutoff_live: f32::from_bits(self.filter_cutoff_live.load(Ordering::Relaxed)),
            osc1_position_live: f32::from_bits(self.osc1_position_live.load(Ordering::Relaxed)),
            osc2_position_live: f32::from_bits(self.osc2_position_live.load(Ordering::Relaxed)),
            active_voice_count: self.active_voice_count.load(Ordering::Relaxed),
            scope_samples,
            scope_sample_count: self.scope_sample_count.load(Ordering::Relaxed),
        }
    }
}

impl Default for WavetableVizState {
    fn default() -> Self {
        Self::new()
    }
}

/// A tear-free snapshot of the viz state, taken by the reader thread.
//
// Fields are pub and will be consumed by the editor in Phase 3. Allow
// dead_code here until then so the crate builds clean without ui feature.
#[allow(dead_code)]
#[derive(Clone)]
pub struct VizSnapshot {
    pub lfo_phases: [f32; 3],
    pub env_amp_value: f32,
    /// 0=Idle, 1=Attack, 2=Decay, 3=Sustain, 4=Release.
    pub env_amp_stage: u32,
    pub env_mod_value: f32,
    pub filter_cutoff_live: f32,
    pub osc1_position_live: f32,
    pub osc2_position_live: f32,
    pub active_voice_count: u32,
    pub scope_samples: [f32; SCOPE_LEN],
    pub scope_sample_count: u32,
}

// ---------------------------------------------------------------------------
// Audio-thread scope ring buffer helper.
// ---------------------------------------------------------------------------

/// Circular buffer that the audio thread fills per-block and periodically
/// publishes to a `WavetableVizState`. This avoids publishing on every single
/// sample — we collect a full SCOPE_LEN worth and then push one atomic
/// snapshot.
pub struct ScopeCollector {
    buf: [f32; SCOPE_LEN],
    write: usize,
    since_publish: u32,
}

impl ScopeCollector {
    pub fn new() -> Self {
        Self {
            buf: [0.0; SCOPE_LEN],
            write: 0,
            since_publish: 0,
        }
    }

    /// Push one stereo frame.
    #[inline]
    pub fn push(&mut self, l: f32, r: f32) {
        self.buf[self.write] = l;
        self.buf[self.write + 1] = r;
        self.write = (self.write + 2) % SCOPE_LEN;
        self.since_publish = self.since_publish.saturating_add(1);
    }

    /// Publish the current ring contents to the shared viz state, flattened
    /// so the reader sees samples in chronological order starting from the
    /// oldest. Call once per audio block.
    pub fn publish(&mut self, viz: &WavetableVizState) {
        if self.since_publish == 0 {
            return;
        }
        // Rotate so that `self.write` becomes the "after newest" position,
        // i.e. samples[0] is the oldest.
        let mut ordered = [0.0f32; SCOPE_LEN];
        let (tail, head) = self.buf.split_at(self.write);
        ordered[..head.len()].copy_from_slice(head);
        ordered[head.len()..].copy_from_slice(tail);
        viz.publish_scope(&ordered, self.since_publish);
        self.since_publish = 0;
    }
}

impl Default for ScopeCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    /// Two-thread stress test: one thread stores scalars and publishes scope
    /// frames at audio-like rates; another reads snapshots at ~60 Hz. Verifies
    /// no tearing in the scope buffer (seq-lock invariant) and that the
    /// sample count is monotonic.
    #[test]
    fn viz_bridge_no_tearing() {
        let viz = Arc::new(WavetableVizState::new());
        let done = Arc::new(AtomicBool::new(false));

        let writer = {
            let viz = viz.clone();
            let done = done.clone();
            thread::spawn(move || {
                let mut collector = ScopeCollector::new();
                let mut t = 0u32;
                while !done.load(Ordering::Acquire) {
                    // Fill a full scope with a recognisable "ramp"
                    // pattern: each frame is (L=t, R=-t).
                    for _ in 0..SCOPE_FRAMES {
                        let v = (t as f32) * 0.001;
                        collector.push(v, -v);
                        t = t.wrapping_add(1);
                    }
                    // Writer-side scalars.
                    viz.store_lfo_phase(0, (t as f32 * 0.0001) % 1.0);
                    viz.store_env_amp((t as f32 * 0.0002) % 1.0, 1);
                    collector.publish(&viz);
                    thread::sleep(Duration::from_micros(500));
                }
            })
        };

        let reader = {
            let viz = viz.clone();
            let done = done.clone();
            thread::spawn(move || -> (bool, u32) {
                let mut last_count = 0u32;
                let mut monotonic = true;
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline && !done.load(Ordering::Acquire) {
                    let snap = viz.read_snapshot();
                    // Tear check: every stereo pair (L, R) must satisfy R == -L
                    // because the writer stores them in that relationship.
                    for pair in snap.scope_samples.chunks_exact(2) {
                        if (pair[0] + pair[1]).abs() > 1e-6 {
                            return (false, snap.scope_sample_count);
                        }
                    }
                    if snap.scope_sample_count < last_count {
                        monotonic = false;
                    }
                    last_count = snap.scope_sample_count;
                    thread::sleep(Duration::from_micros(16_000));
                }
                (monotonic, last_count)
            })
        };

        thread::sleep(Duration::from_millis(500));
        done.store(true, Ordering::Release);
        writer.join().unwrap();
        let (monotonic, final_count) = reader.join().unwrap();

        assert!(monotonic, "scope_sample_count regressed");
        assert!(
            final_count > 0,
            "reader saw zero samples — writer never published"
        );
    }
}
