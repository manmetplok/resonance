//! Backend-agnostic owner of an active audio input stream. Replaces
//! the old `cpal::Stream` field on `RecordingState::input_stream` so
//! the recording / monitor lifecycle works the same regardless of
//! which capture backend is in use.
//!
//! - On non-Linux, only the [`InputHandle::Cpal`] variant exists; the
//!   cpal stream is kept alive until the handle is dropped (matching
//!   cpal's "drop = stop the stream" contract).
//!
//! - On Linux, the dispatcher in [`crate::platform::build_input_stream`]
//!   prefers the native PipeWire backend (`InputHandle::PipeWire`)
//!   because cpal-via-ALSA-via-pipewire-alsa-plugin can't carry more
//!   than two channels through to a pro-audio source. It falls back to
//!   the cpal variant if PipeWire init fails.

#[cfg(target_os = "linux")]
use crate::input_pipewire::PipeWireInputHandle;

/// Owner of a live capture stream. Drop closes the stream cleanly:
/// cpal does so synchronously by stopping the underlying device;
/// PipeWire stops + joins its dedicated thread loop. The inner
/// values are never read directly — the variant exists purely so
/// Drop runs at the right time when the surrounding `Option` is
/// reset to `None`.
#[allow(dead_code)]
pub(crate) enum InputHandle {
    Cpal(cpal::Stream),
    #[cfg(target_os = "linux")]
    PipeWire(PipeWireInputHandle),
}

// SAFETY: Same precedent as `AudioEngine` itself — the handle only
// ever lives on the engine control thread, and Drop runs there. cpal's
// `Stream` is `!Send` on some platforms but is in practice never
// touched from another thread; the PipeWire handle's internal
// `ThreadLoop` owns its own RT thread and cleans up under our drop.
unsafe impl Send for InputHandle {}
