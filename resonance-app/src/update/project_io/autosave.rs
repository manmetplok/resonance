//! Change-gated periodic autosave trigger (ba todo #465, doc #171
//! "Autosave triggering"; code review UPD-07: the write path was merged
//! but nothing ever fired it).
//!
//! The tick consults [`should_autosave`] and, when every precondition
//! holds, starts the autosave snapshot via [`super::start_autosave`]. The
//! decision is a pure function of an [`AutosaveGate`] snapshot so it is
//! testable without booting the app or touching the clock;
//! [`tick_autosave`] samples live state and fires the save.

use std::time::{Duration, SystemTime};

use crate::Resonance;

/// Snapshot of the state the autosave trigger consults on a tick.
#[derive(Debug, Clone, Copy)]
pub struct AutosaveGate {
    /// `autosave.enabled` from the persisted settings.
    pub enabled: bool,
    /// Unsaved changes since the last manual save.
    pub dirty: bool,
    /// The project changed since the last autosave snapshot started (an
    /// unchanged dirty project is not re-snapshotted every interval).
    pub changed_since_autosave: bool,
    /// A project is open (the startup modal has been dismissed).
    pub has_active_project: bool,
    /// Something owns the project right now: a load replaying, an offline
    /// render (bounce, freeze, measurement) or a recording in progress.
    pub busy: bool,
    /// A save is collecting, queued or writing.
    pub save_in_flight: bool,
    /// Now, sampled by the caller.
    pub now: SystemTime,
    /// When the interval started: the later of the moment the project
    /// became dirty and the last autosave.
    pub due_from: SystemTime,
    /// Minimum spacing (`autosave.interval_secs`).
    pub interval: Duration,
}

/// Pure gating predicate: autosave only when enabled, dirty, changed since
/// the last snapshot, a project is open, nothing else owns the project, no
/// save is running, and `interval` has elapsed since `due_from`. A
/// backwards clock jump reads as "not yet elapsed", so a skewed clock can
/// only delay an autosave, never spam one.
pub fn should_autosave(g: AutosaveGate) -> bool {
    g.enabled
        && g.dirty
        && g.changed_since_autosave
        && g.has_active_project
        && !g.busy
        && !g.save_in_flight
        && g
            .now
            .duration_since(g.due_from)
            .is_ok_and(|elapsed| elapsed >= g.interval)
}

/// Called from the tick: sample live state and start an autosave when
/// [`should_autosave`] passes.
pub fn tick_autosave(r: &mut Resonance) {
    if !r.dirty {
        // A manual save (or load) cleaned the project: the next edit
        // starts a fresh interval.
        r.io.autosave_armed_at = None;
        return;
    }
    let now = SystemTime::now();
    let armed_at = *r.io.autosave_armed_at.get_or_insert(now);
    let due_from = r.io.last_autosave_at.map_or(armed_at, |last| last.max(armed_at));
    let cfg = r.autosave_settings();
    let gate = AutosaveGate {
        enabled: cfg.enabled,
        dirty: r.dirty,
        changed_since_autosave: r.io.autosave_revision != Some(r.revision()),
        has_active_project: r.io.has_active_project,
        busy: r.io.loading || r.offline_render_in_progress() || r.transport.recording,
        save_in_flight: r.io.save_state.is_some() || r.io.saving || r.io.manual_save_queued,
        now,
        due_from,
        interval: Duration::from_secs(u64::from(cfg.interval_secs)),
    };
    if should_autosave(gate) {
        let _ = super::start_autosave(r);
        if r.io.save_state.is_some() {
            r.io.autosave_revision = Some(r.revision());
        }
    }
}
