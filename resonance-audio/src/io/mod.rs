//! File-system side of the audio types: reading audio off disk into the
//! shapes the engine plays.
//!
//! Kept out of [`crate::types`] on purpose — the data model must stay
//! usable (and testable) without a filesystem, and format parsing has its
//! own reasons to change.
//!
//! - [`wav`]: memory-mapping a 32-bit float stereo WAV and locating its
//!   PCM data chunk.

pub(crate) mod wav;
