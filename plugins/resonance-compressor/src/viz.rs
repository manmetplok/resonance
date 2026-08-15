//! Shared visualization state between the audio thread and the editor.
//!
//! Every cell is bit-punned atomic — instantaneous input/output/GR
//! values are scalar `AtomicU32`s, and the rolling gain-reduction
//! history is a fixed-length array of atomic samples plus an atomic
//! write index. The audio thread never blocks; the UI reader can
//! tolerate the (very rare) one-sample straddle at frame boundaries
//! since this is purely a viz trace.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

/// Number of samples kept in the GR history ring buffer.
pub const HISTORY_LEN: usize = 256;

/// How often the audio thread pushes a new history sample. One push every
/// `HISTORY_STEP_SAMPLES` at the runtime sample rate; the editor reads and
/// interpolates in its own time. At 48 kHz and 256 samples/step the ring
/// covers ~1.4 seconds at 187 Hz temporal resolution, which looks smooth at
/// 60 FPS.
pub const HISTORY_STEP_SAMPLES: u32 = 256;

/// Which signal the detector is listening to, as observed by the audio
/// thread on its last processed block.
///
/// This is the one thing about a sidechain compressor a user cannot infer
/// from the meters: with a key connected the GR meter is driven by a
/// signal the input meter knows nothing about, so an idle IN meter next
/// to a moving GR meter looks like a bug rather than like ducking working
/// correctly. Naming the source answers "why"; the key meter fed by
/// [`CompressorViz::read_key_db`] answers "by how much".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorSource {
    /// No key connected — the compressor keys off its own input.
    Input,
    /// The host connected an external sidechain key; the detector listens
    /// to the key, and the IN meter shows a signal that is *not* driving
    /// the gain reduction.
    ExternalKey,
}

/// Label for the key meter, which the editor draws only while a key is
/// connected. The `/DET` suffix is the editor's mark for "this bar is
/// what the detector hears": exactly one meter carries it at a time, and
/// with a key connected it moves off the input and onto this one — see
/// [`DetectorSource::input_meter_label`].
pub const KEY_METER_LABEL: &str = "KEY/DET";

impl DetectorSource {
    /// True when the host has an external key wired into the plugin's
    /// sidechain port.
    pub fn key_connected(self) -> bool {
        matches!(self, Self::ExternalKey)
    }

    /// Label for the input meter. Without a key the input *is* the
    /// detector source, and saying so is what keeps "IN" from being read
    /// as "the thing moving the GR meter" once a key takes over.
    pub fn input_meter_label(self) -> &'static str {
        match self {
            Self::Input => "IN/DET",
            Self::ExternalKey => "IN",
        }
    }

    /// One-line statement of what the detector is listening to, for the
    /// editor header.
    pub fn header_text(self) -> &'static str {
        match self {
            Self::Input => "DETECTOR: INPUT",
            Self::ExternalKey => "DETECTOR: SIDECHAIN KEY",
        }
    }
}

pub struct CompressorViz {
    /// Most recent input peak in dBFS (`-inf` when silent).
    pub input_db: AtomicU32,
    /// Most recent output peak in dBFS.
    pub output_db: AtomicU32,
    /// Most recent gain reduction in dB (positive = reducing).
    pub gr_db: AtomicU32,
    /// Whether the last processed block had an external key connected.
    /// Published by the DSP, where the detector source is actually chosen,
    /// so the flag can never disagree with what the detector did.
    pub key_connected: AtomicBool,
    /// Peak detector level across the last block in dBFS — the key,
    /// after the sidechain HPF and the peak/RMS blend, which is the
    /// exact quantity the threshold is compared against and therefore
    /// the one that explains the gain reduction.
    ///
    /// `-inf` when no key is connected. The DSP republishes it every
    /// block, so a disconnected key reads as absent rather than as the
    /// last level it happened to have (ba todo #1342).
    pub key_db: AtomicU32,
    /// Rolling history of GR samples, newest at `write_pos`.
    pub history: GrHistory,
}

pub struct GrHistory {
    samples: [AtomicU32; HISTORY_LEN],
    write_pos: AtomicUsize,
}

impl GrHistory {
    fn new() -> Self {
        Self {
            samples: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            write_pos: AtomicUsize::new(0),
        }
    }

    /// Push one sample. Called from the audio thread once per
    /// `HISTORY_STEP_SAMPLES`; wait-free.
    fn push(&self, v: f32) {
        let pos = self.write_pos.load(Ordering::Relaxed);
        self.samples[pos].store(v.to_bits(), Ordering::Relaxed);
        // Release so the consumer's Acquire on write_pos observes the sample store.
        self.write_pos
            .store((pos + 1) % HISTORY_LEN, Ordering::Release);
    }

    /// Iterate the ring in chronological order (oldest first).
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.write_pos.load(Ordering::Acquire);
        (0..HISTORY_LEN).map(move |i| {
            f32::from_bits(self.samples[(start + i) % HISTORY_LEN].load(Ordering::Relaxed))
        })
    }
}

impl CompressorViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            input_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            output_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            gr_db: AtomicU32::new(0.0f32.to_bits()),
            key_connected: AtomicBool::new(false),
            key_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            history: GrHistory::new(),
        })
    }

    /// Publish whether an external key is feeding the detector. Called
    /// once per block from the audio thread; wait-free.
    pub fn store_key_connected(&self, connected: bool) {
        self.key_connected.store(connected, Ordering::Relaxed);
    }

    /// Publish the block's meter levels. `key_db` is `-inf` when no key
    /// is connected; it travels with the others so the key level can
    /// never lag a block behind the input it is being compared to.
    pub fn store_levels(&self, input_db: f32, output_db: f32, gr_db: f32, key_db: f32) {
        self.input_db.store(input_db.to_bits(), Ordering::Relaxed);
        self.output_db.store(output_db.to_bits(), Ordering::Relaxed);
        self.gr_db.store(gr_db.to_bits(), Ordering::Relaxed);
        self.key_db.store(key_db.to_bits(), Ordering::Relaxed);
    }

    pub fn push_gr(&self, gr_db: f32) {
        self.history.push(gr_db);
    }

    pub fn read_input_db(&self) -> f32 {
        f32::from_bits(self.input_db.load(Ordering::Relaxed))
    }

    pub fn read_output_db(&self) -> f32 {
        f32::from_bits(self.output_db.load(Ordering::Relaxed))
    }

    pub fn read_gr_db(&self) -> f32 {
        f32::from_bits(self.gr_db.load(Ordering::Relaxed))
    }

    /// The key's level on the last processed block, or `None` when no
    /// key is connected.
    ///
    /// One accessor rather than a bare float so the editor cannot draw a
    /// key meter for a key that is not there: absence is the DSP's
    /// `-inf`, published every block from the same place as the presence
    /// bit, not a value the reader has to remember to ignore.
    pub fn read_key_db(&self) -> Option<f32> {
        let db = f32::from_bits(self.key_db.load(Ordering::Relaxed));
        db.is_finite().then_some(db)
    }

    /// What the detector was listening to on the last processed block.
    pub fn detector_source(&self) -> DetectorSource {
        if self.key_connected.load(Ordering::Relaxed) {
            DetectorSource::ExternalKey
        } else {
            DetectorSource::Input
        }
    }
}
