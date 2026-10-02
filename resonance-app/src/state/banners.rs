//! What the window's status area at the top shows (ARCH-06 A6-2): the
//! transient, dismissable error banner, and two persistent statuses that
//! no error may overwrite — the audio engine's health and a run of
//! autosave failures (code review UX-04, UX-13).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about the banner can take `&Banners` / `&mut Banners`
//! instead of the whole app.

/// How many autosaves in a row must fail before the indicator shows. One
/// miss is often transient (a file briefly locked); a run of them (a full
/// disk, a revoked permission) means nothing is being snapshotted.
pub const AUTOSAVE_FAILURES_BEFORE_INDICATOR: u32 = 3;

/// The audio engine's health as the tick handler last polled it
/// (`update::tick::poll_engine_health`). A *status*, not a message: it is
/// derived from the engine every tick, cannot be dismissed, and clears by
/// itself when the condition does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EngineHealth {
    #[default]
    Ok,
    /// The output stream is gone (device unplugged, audio server
    /// restarted) while the engine thread lives on. Recovers when the
    /// backend reconnects.
    StreamLost,
    /// The engine thread is gone: every command is silently dropped.
    /// Never recovers.
    Disconnected,
}

impl EngineHealth {
    /// The status line shown while the engine is unhealthy.
    pub fn message(self) -> Option<&'static str> {
        match self {
            Self::Ok => None,
            Self::StreamLost => Some(
                "Audio output stream lost (device unplugged or audio server restarted) — \
                 playback and recording are silent until it reconnects",
            ),
            Self::Disconnected => Some(
                "Audio engine stopped responding — restart the app; edits are no longer \
                 reaching audio",
            ),
        }
    }
}

/// The window's status area (`view::mod::view_status_area`).
#[derive(Debug, Clone, Default)]
pub struct Banners {
    /// The transient error banner, if any is showing. Anything in this
    /// crate that wants to surface a message to the user sets this
    /// directly; a later message replaces it, and the user dismisses it.
    /// Never used for the persistent statuses below.
    pub error_message: Option<String>,
    /// Persistent engine/stream health, shown on its own line above the
    /// error banner (code review UX-04).
    pub engine_health: EngineHealth,
    /// Autosaves failed in a row since the last successful save.
    pub autosave_failures: u32,
    /// The latest autosave failure's reason.
    pub autosave_failure: Option<String>,
    /// What the last undo / redo did ("Undid delete bus"), shown briefly
    /// beside the project title so a history step is never silent — it
    /// can stop the transport and change a tab the user isn't looking at
    /// (code review UX-12). Expired by the tick after
    /// [`HISTORY_NOTICE_DURATION`].
    pub history_notice: Option<HistoryNotice>,
}

/// How long the undo / redo notice stays up.
pub const HISTORY_NOTICE_DURATION: std::time::Duration = std::time::Duration::from_millis(2500);

/// A short-lived undo / redo notice (see [`Banners::history_notice`]).
#[derive(Debug, Clone)]
pub struct HistoryNotice {
    pub text: String,
    pub shown_at: std::time::Instant,
}

impl Banners {
    /// Count one more failed autosave (code review UX-13).
    pub fn note_autosave_failure(&mut self, reason: String) {
        self.autosave_failures = self.autosave_failures.saturating_add(1);
        self.autosave_failure = Some(reason);
    }

    /// A save landed: the location takes writes again.
    pub fn clear_autosave_failures(&mut self) {
        self.autosave_failures = 0;
        self.autosave_failure = None;
    }

    /// Show `text` as the undo / redo notice, replacing any earlier one.
    pub fn note_history(&mut self, text: String) {
        self.history_notice = Some(HistoryNotice {
            text,
            shown_at: std::time::Instant::now(),
        });
    }

    /// Drop the undo / redo notice once it has been up long enough.
    pub fn expire_history_notice(&mut self, now: std::time::Instant) {
        if self
            .history_notice
            .as_ref()
            .is_some_and(|n| now.saturating_duration_since(n.shown_at) >= HISTORY_NOTICE_DURATION)
        {
            self.history_notice = None;
        }
    }

    /// The persistent "autosave failing" line, once enough misses ran in
    /// a row.
    pub fn autosave_failing(&self) -> Option<String> {
        if self.autosave_failures < AUTOSAVE_FAILURES_BEFORE_INDICATOR {
            return None;
        }
        let reason = self.autosave_failure.as_deref().unwrap_or("unknown error");
        Some(format!("Autosave failing: {reason}"))
    }
}
