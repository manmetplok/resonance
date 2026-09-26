//! The transient error/notification banner shown at the top of the
//! window, plus the latches that decide when the tick handler may
//! (re-)raise it (ARCH-06 A6-2).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about the banner can take `&Banners` / `&mut Banners`
//! instead of the whole app.

/// The user-facing error/notification banner (`view::mod::view`) and the
/// two latches that stop the tick handler from re-raising a dismissed
/// one every frame.
#[derive(Debug, Clone, Default)]
pub struct Banners {
    /// The banner text, if any is showing. Anything in this crate that
    /// wants to surface a message to the user sets this directly.
    pub error_message: Option<String>,
    /// Set once the tick handler has surfaced the "engine stopped
    /// responding" banner for [`resonance_audio::AudioEngine::is_disconnected`]
    /// (see `update::tick::check_engine_disconnected`). Latches the check
    /// app-side so a dismissed (or superseded) banner isn't forced back
    /// onto `error_message` every subsequent tick — the underlying engine
    /// latch never resets, so without this the message would be
    /// unclearable.
    pub engine_disconnected_banner_shown: bool,
    /// Set while the tick handler is showing the "audio stream lost"
    /// banner for [`resonance_audio::AudioEngine::output_stream_lost`]
    /// (see `update::tick::check_output_stream_lost`). Unlike the
    /// engine-death latch above, this one clears again: the PipeWire
    /// backend reports the stream coming back, and the tick handler
    /// then removes the banner it raised (and only that banner).
    pub stream_lost_banner_shown: bool,
}
