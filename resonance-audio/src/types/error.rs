//! Typed engine failure carried by [`AudioEvent::Error`](super::AudioEvent::Error)
//! (ARCH-05 / epic C, ba doc `refactor-intent.md` §"Epic C").
//!
//! Before this, every engine failure that didn't already have its own typed
//! event (`ExportError`, `PluginLoadFailed`, ...) reached the app as a bare
//! `AudioEvent::Error(String)` — fine for the banner, which only ever showed
//! the text, but it gave a control-API job (`render.*`, `vocal.render`, ...)
//! nothing to branch on beyond string-matching. `kind` fixes that; `message`
//! is unchanged and is still what the app's banner shows
//! (`engine_events/dispatch.rs` -> `transport::error`).
//!
//! `EngineErrorKind` mirrors the applicable variants of
//! `resonance_control::rpc::ErrorKind` (`NotFound`, `Busy`, `Unsupported`)
//! plus two engine-specific kinds (`Io`, `Plugin`). It is a hand-kept mirror,
//! not a dependency: `resonance-audio` sits below `resonance-control` in the
//! crate DAG (see `ARCHITECTURE.md`) and must not depend on it.

use std::fmt;

/// Stable category for an [`EngineError`]. A programmatic consumer (a
/// failed control-API job, eventually) branches on this; a human reads
/// `message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineErrorKind {
    /// A referenced entity (track, clip, plugin instance, bus, ...) does
    /// not exist in the engine.
    NotFound,
    /// The engine cannot service the request right now — a conflicting
    /// operation is in flight, or the transport state forbids it.
    Busy,
    /// The operation, format, or device isn't supported (missing CLAP
    /// extension, wrong track kind, unsupported sample format, ...).
    Unsupported,
    /// A filesystem or OS I/O failure (open, read, write, create dir, ...).
    Io,
    /// A CLAP plugin instance failed to load, activate, or otherwise
    /// misbehaved.
    Plugin,
    /// An unexpected internal failure with no more specific classification.
    /// Left honest rather than guessed at a kind that doesn't fit.
    Internal,
}

/// A typed engine failure. Replaces the historical
/// `AudioEvent::Error(String)`: `kind` is new, `message` is the same text
/// that was always there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    pub kind: EngineErrorKind,
    pub message: String,
}

impl EngineError {
    pub fn new(kind: EngineErrorKind, message: impl Into<String>) -> Self {
        EngineError {
            kind,
            message: message.into(),
        }
    }

    /// A referenced entity does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(EngineErrorKind::NotFound, message)
    }

    /// The engine cannot service the request right now.
    pub fn busy(message: impl Into<String>) -> Self {
        Self::new(EngineErrorKind::Busy, message)
    }

    /// The operation, format, or device isn't supported.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(EngineErrorKind::Unsupported, message)
    }

    /// A filesystem or OS I/O failure.
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(EngineErrorKind::Io, message)
    }

    /// A CLAP plugin instance failed to load, activate, or otherwise
    /// misbehaved.
    pub fn plugin(message: impl Into<String>) -> Self {
        Self::new(EngineErrorKind::Plugin, message)
    }

    /// An unexpected internal failure with no more specific classification.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(EngineErrorKind::Internal, message)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// `resonance_common::WavDecodeError` (C-4) always classifies as
/// [`EngineErrorKind::Io`] — matches the pre-existing precedent at every
/// `decode_file`/`decode_wav_*` call site in this crate (`clips.rs`,
/// `audition.rs`), which already wrapped the bare decode-failure string in
/// `EngineError::io(...)` regardless of which decode step failed.
impl From<resonance_common::WavDecodeError> for EngineError {
    fn from(e: resonance_common::WavDecodeError) -> Self {
        EngineError::io(e.to_string())
    }
}

/// `resonance_common::AudioProbeError` (C-4) also classifies as
/// [`EngineErrorKind::Io`] — same precedent as [`EngineError`]'s
/// `WavDecodeError` conversion above; `import_pool.rs`'s `probe_audio_file`
/// call currently stringifies this itself (its enclosing fn predates a
/// typed `EngineError` return), but a future caller that already returns
/// `Result<_, EngineError>` can use `?` directly.
impl From<resonance_common::AudioProbeError> for EngineError {
    fn from(e: resonance_common::AudioProbeError) -> Self {
        EngineError::io(e.to_string())
    }
}
