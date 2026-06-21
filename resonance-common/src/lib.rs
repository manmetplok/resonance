/// Shared utilities for Resonance plugins.
mod denormal;
pub mod drum_map;
pub mod freeze;
pub mod registry;
mod scan;
mod wav;

pub use denormal::flush_denormals;
pub use scan::scan_directory;
pub use wav::{
    decode_file, decode_wav_channels, decode_wav_stereo, linear_resample_mono,
    linear_resample_stereo, StreamingLinearResampler, WavChannels,
};
pub use freeze::{
    compute_fingerprint, FreezeCacheRef, FreezeCacheStatus, FreezeFingerprintBuilder,
    FreezeFingerprintInputs, TrackFreezeState,
};
