//! Drum kit data model: a pad holds one bank per loaded mic position
//! (close mics per drum group, plus a shared overhead bank) and the
//! samples for each bank are organised as velocity layers with
//! per-layer round-robin takes.

/// A single decoded stereo sample ready for playback.
pub struct LoadedSample {
    /// Stereo interleaved f32 at the host sample rate.
    pub data: Vec<f32>,
    /// Number of stereo frames.
    pub frames: usize,
}

/// All round-robin takes recorded at a given velocity.
pub struct VelocityLayer {
    pub round_robins: Vec<LoadedSample>,
}

/// One mic position's sample bank for a single pad. The plugin loads a
/// separate `LoadedMicBank` per position the library provides for that
/// pad (e.g. `KickIn`, `KickOut`, `OHsAB`), so multiple voices can be
/// triggered simultaneously on note-on and routed to different output
/// ports.
pub struct LoadedMicBank {
    /// Canonical position key from the manifest (e.g. `"KickIn"`, `"OHsAB"`).
    /// Empty for the embedded-fallback bank used by pads with no manifest
    /// match (Clap, Cowbell). Consumed by the Phase 6 editor for display.
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
#[allow(dead_code)] // consulted by external tooling / tests
pub const NUM_OUTPUT_PORTS: usize = 7;
pub const OVERHEAD_PORT_INDEX: usize = 6;

/// A loaded pad with one or more mic banks. Kick and snare each get two
/// close banks (in/out and top/btm); toms and hats get one; cymbals get
/// none. Every pad (except the embedded fallback path) also gets an
/// overhead bank that accumulates into the shared Overhead port.
pub struct LoadedPad {
    /// Display name, sourced from `PAD_MAPPINGS`.
    #[allow(dead_code)]
    pub name: String,
    pub choke_group: Option<u8>,
    /// Which close-mic output port this pad's close signal routes to.
    /// Hardcoded per pad slot — see `PAD_MAPPINGS`.
    pub output_group: OutputGroup,
    /// Close-mic banks, one per position the library supplies for this
    /// pad. Empty for cymbal-class pads on Drummica (the library has no
    /// cymbal close mics) and for Clap / Cowbell (no manifest match).
    /// For Clap / Cowbell this vec holds a single pseudo-bank built
    /// from the embedded fallback sample so the pad still makes sound.
    pub close_mics: Vec<LoadedMicBank>,
    /// Overhead mic bank. `None` when the library ships no overhead
    /// recording for this pad, or when the pad uses the embedded
    /// fallback path (Clap / Cowbell).
    pub overhead: Option<LoadedMicBank>,
}

impl LoadedSample {
    pub fn from_data(data: Vec<f32>) -> Self {
        let frames = data.len() / 2;
        Self { data, frames }
    }
}


/// Decode a WAV file from a byte slice into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    resonance_common::decode_wav_stereo(data, target_sample_rate)
}
