//! Drum kit data model: a pad holds one bank per loaded mic position
//! (close mics per drum group, plus a shared overhead bank) and the
//! samples for each bank are organised as velocity layers with
//! per-layer round-robin takes.

use std::path::Path;
use std::sync::Arc;

use crate::stream::TailSource;

/// The decoded audio of one take, at the host rate. Immutable once built
/// and shared through `Arc`: the process-wide sample cache
/// ([`crate::kit_loader::cache`]) hands the same `SampleData` to every
/// kit — and every plugin instance — that loads the same file at the
/// same rate.
///
/// Mono files stay mono (E5): `channels` is 1 or 2, and a reader plays a
/// mono take on both sides ([`SampleData::frame`]), which gives exactly
/// the floats the old duplicate-to-stereo decode did.
///
/// The take is `frames` long, of which the first `resident_frames` are in
/// memory. A take decoded with a preload (disk streaming, E14) keeps only
/// that **head** resident when it is longer; the rest — its [`tail`] — is
/// read from the file while a voice plays ([`crate::stream`]). Everything
/// that reads samples goes through `resident_frames` / [`samples`] for
/// the head.
///
/// [`samples`]: SampleData::samples
/// [`tail`]: SampleData::tail
pub struct SampleData {
    /// Interleaved, `channels` per frame, `resident_frames` frames.
    samples: Box<[f32]>,
    /// 1 (mono) or 2 (stereo).
    channels: usize,
    /// Frames in the whole take.
    frames: usize,
    /// Where frames `resident_frames..frames` are, for a streamed take.
    tail: Option<Arc<TailSource>>,
}

impl SampleData {
    /// A take from interleaved samples with `channels` (1 or 2) per frame.
    /// A trailing partial frame is dropped.
    pub fn new(mut samples: Vec<f32>, channels: usize) -> Self {
        let channels = channels.clamp(1, 2);
        let frames = samples.len() / channels;
        samples.truncate(frames * channels);
        Self {
            samples: samples.into_boxed_slice(),
            channels,
            frames,
            tail: None,
        }
    }

    /// A streamed take: `head` resident (interleaved, `channels` per
    /// frame), the take `frames` long in all, the rest read from `tail`.
    pub fn split(head: Vec<f32>, channels: usize, frames: usize, tail: Arc<TailSource>) -> Self {
        let mut data = Self::new(head, channels);
        data.frames = frames.max(data.frames);
        data.tail = Some(tail);
        data
    }

    /// Where the frames past the head are, for a streamed take; `None`
    /// when the whole take is resident.
    pub fn tail(&self) -> Option<&Arc<TailSource>> {
        self.tail.as_ref()
    }

    /// A mono take.
    pub fn mono(samples: Vec<f32>) -> Self {
        Self::new(samples, 1)
    }

    /// A stereo-interleaved take.
    pub fn stereo(samples: Vec<f32>) -> Self {
        Self::new(samples, 2)
    }

    /// 1 for a mono take, 2 for stereo.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Length of the whole take, in frames.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Frames held in memory, from the start of the take. Equal to
    /// [`frames`](Self::frames) unless disk streaming (E14) split the
    /// take into a resident head and a streamed tail.
    pub fn resident_frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    /// The resident samples, interleaved `channels()` per frame.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Bytes of sample memory this take holds.
    pub fn bytes(&self) -> usize {
        std::mem::size_of_val(&*self.samples)
    }

    /// Resident frame `i` as (left, right); a mono take gives its one
    /// channel on both sides. Panics past `resident_frames`.
    #[inline]
    pub fn frame(&self, i: usize) -> (f32, f32) {
        let idx = i * self.channels;
        (self.samples[idx], self.samples[idx + self.channels - 1])
    }
}

/// One round-robin take as a kit holds it: a shared handle on decoded
/// [`SampleData`]. Cloning it is an `Arc` bump, which is what lets a kit
/// rebuild reuse the pads it did not change (E4).
#[derive(Clone)]
pub struct LoadedSample {
    data: Arc<SampleData>,
}

/// All round-robin takes recorded at a given velocity.
#[derive(Clone)]
pub struct VelocityLayer {
    pub round_robins: Vec<LoadedSample>,
}

/// One mic position's sample bank for a single pad. The plugin loads a
/// separate `LoadedMicBank` per position the library provides for that
/// pad (e.g. `KickIn`, `KickOut`, `OHsAB`), so multiple voices can be
/// triggered simultaneously on note-on and routed to different output
/// ports.
#[derive(Clone)]
pub struct LoadedMicBank {
    /// Canonical position key from the manifest (e.g. `"KickIn"`, `"OHsAB"`).
    /// `"fallback"` for the one bank of a built-in-kit pad.
    #[allow(dead_code)]
    pub position: String,
    /// The manifest setup key that produced this bank (e.g.
    /// `"01_KickIn_e901"`). Persists through plugin state so loading the
    /// same kit restores the exact mic brand/model the user chose.
    #[allow(dead_code)]
    pub setup_key: String,
    /// Velocity layers sorted soft → loud.
    pub layers: Vec<VelocityLayer>,
}

/// Classification of which plugin output port a pad's close-mic signal
/// feeds. Overhead is not part of this enum — it's a mic type rather
/// than an assignable group and has its own dedicated output port.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputGroup {
    Main = 0,
    Kick = 1,
    Snare = 2,
    Toms = 3,
    Hats = 4,
    Cymbals = 5,
}

impl OutputGroup {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Total number of stereo output ports the drum plugin declares.
/// Ports 0..5 correspond to `OutputGroup` variants; port 6 is Overhead.
pub const NUM_OUTPUT_PORTS: usize = 7;
pub const OVERHEAD_PORT_INDEX: usize = 6;

/// Names of the stereo output ports, in port order. Index 0..=5 match the
/// `OutputGroup` discriminants; index 6 is the shared Overhead bus.
///
/// Single source of truth: `ResonanceDrums::output_layout` builds the CLAP
/// port list from this array and the editor's KIT card reads it back, so the
/// UI can never claim a routing mode the plugin does not declare.
pub const OUTPUT_PORT_NAMES: [&str; NUM_OUTPUT_PORTS] = [
    "Main", "Kick", "Snare", "Toms", "Hats", "Cymbals", "Overhead",
];

/// Truthful one-line summary of the plugin's output routing.
///
/// The plugin declares every port in [`OUTPUT_PORT_NAMES`] unconditionally —
/// there is no stereo-only mode and no parameter that switches one — so the
/// editor renders this as a static readout rather than a toggle.
pub fn routing_summary() -> String {
    format!("Multi-out · {NUM_OUTPUT_PORTS} ports")
}

/// The port list as a single comma-separated line, for the KIT card readout.
pub fn routing_port_list() -> String {
    OUTPUT_PORT_NAMES.join(" · ")
}

/// A loaded pad with one or more mic banks. Kick and snare each get two
/// close banks (in/out and top/btm); toms and hats get one; cymbals get
/// none. Every pad (except the embedded fallback path) also gets an
/// overhead bank that accumulates into the shared Overhead port.
/// A pad whose piece the kit lacks has no banks at all and is silent.
///
/// `Clone` copies the bank structure and bumps every take's `Arc`; no
/// sample memory is copied.
#[derive(Clone)]
pub struct LoadedPad {
    /// Display name: the kit's (`_meta.pieces`, see [`crate::pad_map`]), or
    /// the GM pad name for the built-in kit.
    #[allow(dead_code)]
    pub name: String,
    pub choke_group: Option<u8>,
    /// Which close-mic output port this pad's close signal routes to.
    /// Hardcoded per pad slot — see `PAD_MAPPINGS`.
    pub output_group: OutputGroup,
    /// Close-mic banks, one per position the library supplies for this
    /// pad. Empty for cymbal-class pads on Drummica (the library has no
    /// cymbal close mics) and for a pad whose piece the kit lacks (D7:
    /// silent). A built-in-kit pad holds one pseudo-bank of its embedded
    /// sample.
    pub close_mics: Vec<LoadedMicBank>,
    /// Overhead mic bank. `None` when the library ships no overhead
    /// recording for this pad, when the kit lacks the pad's piece, and on
    /// the built-in kit.
    pub overhead: Option<LoadedMicBank>,
}

impl LoadedSample {
    /// A stereo take from interleaved samples (the historical shape).
    pub fn from_data(data: Vec<f32>) -> Self {
        Self::from_shared(Arc::new(SampleData::stereo(data)))
    }

    /// A mono take.
    pub fn mono(data: Vec<f32>) -> Self {
        Self::from_shared(Arc::new(SampleData::mono(data)))
    }

    /// A take on already-shared sample data (the cache's).
    pub fn from_shared(data: Arc<SampleData>) -> Self {
        Self { data }
    }

    /// The shared sample data.
    pub fn shared(&self) -> &Arc<SampleData> {
        &self.data
    }
}

/// Reads go straight to the sample data: `take.frames()`,
/// `take.frame(i)`, `take.channels()`.
impl std::ops::Deref for LoadedSample {
    type Target = SampleData;
    fn deref(&self) -> &SampleData {
        &self.data
    }
}

/// Decode a WAV file from a byte slice into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    resonance_common::decode_wav_stereo(data, target_sample_rate).map_err(|e| e.to_string())
}

/// Decode a WAV file into a take at the target rate, keeping mono files
/// mono (E5).
pub fn decode_sample(data: Vec<u8>, target_sample_rate: f32) -> Result<SampleData, String> {
    let decoded =
        resonance_common::decode_wav_native(data, target_sample_rate).map_err(|e| e.to_string())?;
    Ok(SampleData::new(decoded.samples, decoded.channels))
}

/// [`decode_sample`] of the file at `path` (whose bytes `data` are, and
/// which is `file_len` long and was last modified at `modified`, as it
/// was stated before the read), keeping only the first `preload` frames
/// resident when the take is longer than that by at least
/// [`crate::stream::MIN_STREAMED_TAIL`] — the rest streams from the file
/// (E14). `preload == 0` keeps every take whole. The frames are the same
/// either way, bit for bit.
pub fn decode_sample_streamed(
    data: Vec<u8>,
    target_sample_rate: f32,
    preload: u32,
    path: &Path,
    file_len: u64,
    modified: Option<std::time::SystemTime>,
) -> Result<SampleData, String> {
    if preload == 0 {
        return decode_sample(data, target_sample_rate);
    }
    let split = resonance_common::decode_wav_split(
        data,
        target_sample_rate,
        preload as usize,
        crate::stream::MIN_STREAMED_TAIL,
    )
    .map_err(|e| e.to_string())?;
    Ok(match split.tail {
        None => SampleData::new(split.samples, split.channels),
        Some(tail) => SampleData::split(
            split.samples,
            split.channels,
            split.frames,
            Arc::new(TailSource {
                path: path.to_path_buf(),
                file_len,
                modified,
                tail,
            }),
        ),
    })
}
