//! The periodic `Tick` handler: drains engine events, decays VU meter
//! levels, syncs the tempo display, follows the playhead, and
//! re-enumerates MIDI devices.
//!
//! The tick runs at two rates (see [`tick_interval`]): the fast frame
//! rate while anything animates or a job is in flight, and a slow idle
//! rate otherwise. The slow tick still drains engine events, so nothing
//! is ever starved — it just stops the app redrawing at 60 Hz while it
//! sits fully idle.
use iced::Task;

use crate::message::Message;
use crate::Resonance;
use resonance_audio::types::{AudioCommand, BusId, TrackId};

/// VU-meter peak decay factor applied per frame tick.
pub const PEAK_DECAY: f32 = 0.85;

/// Floor below which a decaying VU level snaps to exactly `0.0`.
///
/// `1e-4` linear is -80 dBFS — far below anything a meter draws. Without
/// the floor the exponential decay churns through subnormals for ~10 s
/// after silence, keeping every meter canvas cache invalidating (and the
/// idle-rate gate below never engaging, since levels never *reach* zero).
pub const PEAK_SETTLE_EPSILON: f32 = 1e-4;

/// Slow tick interval (ms) used while the app is fully idle. Engine
/// events (plugin latency changes, background job completions, control
/// acks…) are still drained at this rate; the moment one of them — or
/// any user action — makes [`needs_fast_tick`] true, iced re-diffs the
/// subscription and the fast tick re-arms.
pub const TICK_INTERVAL_IDLE_MS: u64 = 200;

/// The interval the `Tick` subscription should run at for the current
/// state: the frame rate while active, [`TICK_INTERVAL_IDLE_MS`] while
/// idle. Pure so the subscription stays a one-liner and tests can assert
/// on the chosen rate directly.
pub fn tick_interval(r: &Resonance) -> std::time::Duration {
    let ms = if needs_fast_tick(r) {
        crate::update::TICK_INTERVAL_MS
    } else {
        TICK_INTERVAL_IDLE_MS
    };
    std::time::Duration::from_millis(ms)
}

/// Whether anything the tick services currently needs the frame rate.
///
/// What the tick services, and the rate each needs:
/// * engine-event drain — periodic always (the slow tick covers idle);
///   fast while the transport runs so playhead/meter events land smoothly;
/// * VU decay + `PollPeaks` — fast while meters are live (playing,
///   recording, unsettled levels, armed/monitored inputs, audition);
/// * A/B reference metering — fast while the reference rail polls;
/// * tempo/time-signature sync + playhead auto-follow — playing only;
/// * MIDI device re-enumeration — 2 s cadence; the idle tick suffices.
///
/// Conservative by design: renders, freezes, bounces, plugin scans, relink
/// folder searches (for their progress count) and
/// live control jobs all hold the fast rate so their progress events are
/// drained promptly. When in doubt, stay fast — the cost of a wrong
/// `true` is the old always-on behavior.
fn needs_fast_tick(r: &Resonance) -> bool {
    r.transport.playing
        || r.transport.recording
        || meters_unsettled(r)
        || input_monitoring_active(r)
        || r.browser.audition.playing.is_some()
        || r.freeze.any_in_flight()
        || r.io.bouncing
        || r.io.loading
        || r.modals.bounce_in_progress.is_some()
        || export_render_in_flight(r)
        || (r.mixer.reference_panel_open && !r.reference.entries.is_empty())
        || r.plugin_catalog.plugin_scan_in_progress
        || r.relink.scanning()
        || !r.control.pending_tracks.is_empty()
        || r.control.jobs.has_live_offline_measure()
}

/// Any VU level not yet settled to exactly `0.0`. This is the
/// load-bearing meter condition: the transport bar's master meter is
/// always visible, so "someone consumes peaks" reduces to "the meters
/// have not finished falling" once the transport is stopped.
fn meters_unsettled(r: &Resonance) -> bool {
    r.master.level_l != 0.0
        || r.master.level_r != 0.0
        || r.registry
            .tracks
            .iter()
            .any(|t| t.level_l != 0.0 || t.level_r != 0.0)
        || r.registry
            .busses
            .iter()
            .any(|b| b.level_l != 0.0 || b.level_r != 0.0)
}

/// A record-armed or input-monitoring track can move the meters while
/// the transport is stopped, so peaks must keep being polled for it.
fn input_monitoring_active(r: &Resonance) -> bool {
    r.registry
        .tracks
        .iter()
        .any(|t| t.record_armed || t.monitor_enabled)
}

/// An export-dialog stem/mixdown render is in flight.
fn export_render_in_flight(r: &Resonance) -> bool {
    matches!(
        r.modals.export_dialog.as_ref().map(|d| &d.phase),
        Some(crate::state::ExportPhase::Rendering { .. })
    )
}

/// Handle the per-frame subscription tick.
pub fn handle_tick(r: &mut Resonance) -> Task<Message> {
    let mut tasks = Vec::new();
    while let Some(event) = r.engine.try_recv() {
        let task = crate::engine_events::handle_engine_event(r, event);
        tasks.push(task);
    }
    update_vu_meters(r);
    poll_ab_meters(r);
    sync_tempo_at_playhead(r);
    if let Some(task) = follow_playhead(r) {
        tasks.push(task);
    }
    refresh_midi_devices_if_stale(r);
    check_engine_disconnected(r);
    check_output_stream_lost(r);
    // Change-gated periodic autosave (todo #465, code review UPD-07).
    crate::update::project_io::tick_autosave(r);
    if tasks.is_empty() {
        Task::none()
    } else {
        Task::batch(tasks)
    }
}

/// Surface the engine's death to the user. `AudioEngine::is_disconnected`
/// latches true forever the first time any `send` finds the command
/// channel gone (the engine thread exited or panicked) — from that
/// moment every `let _ = r.engine.send(...)` call site in the app
/// (there are dozens) silently drops its command, including MCP-driven
/// edits that still ack success back to the caller. Checked once here
/// (every tick already drains engine events, so nothing is missed) and
/// latched app-side via `engine_disconnected_banner_shown` so the
/// standard error banner is set exactly once instead of being forced
/// back onto `error_message` on every subsequent tick.
fn check_engine_disconnected(r: &mut Resonance) {
    if r.banners.engine_disconnected_banner_shown {
        return;
    }
    if r.engine.is_disconnected() {
        r.banners.engine_disconnected_banner_shown = true;
        r.banners.error_message = Some(
            "Audio engine stopped responding — restart the app; edits are no longer reaching audio"
                .to_string(),
        );
    }
}

/// User-facing banner for a lost output stream. A named constant so the
/// recovery branch of [`check_output_stream_lost`] can clear exactly the
/// banner this module raised and never an unrelated error that landed on
/// `error_message` in the meantime.
const STREAM_LOST_BANNER: &str = "Audio output stream lost (device unplugged or audio server \
     restarted) — playback and recording are silent until it reconnects";

/// Surface output-*stream* death to the user. Distinct from
/// [`check_engine_disconnected`]: when the USB interface is unplugged or
/// PipeWire restarts, the engine thread stays alive — the transport
/// appears to run and edits still ack — so `is_disconnected` never
/// fires, while no audio plays and recording captures nothing. The
/// output backends publish that state as a per-engine flag
/// (`AudioEngine::output_stream_lost`), polled here into the same
/// persistent error banner engine death uses.
///
/// Unlike engine death this state can recover: PipeWire reconnecting
/// the stream to a new sink clears the flag, and this check then clears
/// the banner it raised — but only if `error_message` still holds
/// exactly [`STREAM_LOST_BANNER`], so a different error that arrived in
/// the meantime is left standing. On the cpal fallback backend recovery
/// is not observable and the banner stays until restart.
/// `stream_lost_banner_shown` tracks the raise so a banner the user
/// dismissed isn't forced back every tick while the flag stays set.
fn check_output_stream_lost(r: &mut Resonance) {
    // Engine death outranks stream loss: once the engine thread is gone
    // the stream banner would understate the failure (nothing recovers a
    // dead engine thread), so never raise over that banner.
    if r.banners.engine_disconnected_banner_shown {
        return;
    }
    let lost = r.engine.output_stream_lost();
    if lost && !r.banners.stream_lost_banner_shown {
        r.banners.stream_lost_banner_shown = true;
        r.banners.error_message = Some(STREAM_LOST_BANNER.to_string());
    } else if !lost && r.banners.stream_lost_banner_shown {
        r.banners.stream_lost_banner_shown = false;
        if r.banners.error_message.as_deref() == Some(STREAM_LOST_BANNER) {
            r.banners.error_message = None;
        }
    }
}

/// Re-enumerate hardware MIDI ports periodically so a freshly
/// plugged controller appears in pickers without a restart.
/// Cadence is intentionally low (every 2 s) — ALSA seq enumeration
/// is cheap, but doing it every frame would still be wasteful.
fn refresh_midi_devices_if_stale(r: &mut Resonance) {
    if r.midi_devices.midi_devices_last_refresh.elapsed() < std::time::Duration::from_secs(2) {
        return;
    }
    r.midi_devices.midi_devices_last_refresh = std::time::Instant::now();
    let _ = r.engine.send(AudioCommand::ListMidiInputDevices);
    let _ = r.engine.send(AudioCommand::ListMidiOutputDevices);
}

/// Per-tick VU step: decay current levels and — while anything consumes
/// them — ask the engine for a fresh peak snapshot. The reply arrives on
/// a later tick as `AudioEvent::PeakSnapshot` and is folded in by
/// `apply_peak_snapshot`. Splitting the read across two ticks is fine for
/// a meter; it keeps the GUI thread from contending on engine RwLocks.
///
/// Decayed levels snap to exactly `0.0` below [`PEAK_SETTLE_EPSILON`], so
/// meters settle instead of churning through subnormals, and the poll is
/// skipped once the transport is stopped, nothing is armed/monitoring/
/// auditioning, and every meter has settled — a fully idle app sends no
/// per-tick engine traffic for peaks.
fn update_vu_meters(r: &mut Resonance) {
    for track in &mut r.registry.tracks {
        decay_level(&mut track.level_l);
        decay_level(&mut track.level_r);
    }
    for bus in &mut r.registry.busses {
        decay_level(&mut bus.level_l);
        decay_level(&mut bus.level_r);
    }
    decay_level(&mut r.master.level_l);
    decay_level(&mut r.master.level_r);
    if peaks_have_consumers(r) {
        let _ = r.engine.send(AudioCommand::PollPeaks);
    }
}

/// One decay step with the settle floor.
fn decay_level(level: &mut f32) {
    *level *= PEAK_DECAY;
    if *level < PEAK_SETTLE_EPSILON {
        *level = 0.0;
    }
}

/// Whether a `PollPeaks` round-trip would feed anything: audio is (or
/// may be) sounding, or a meter is still falling toward zero.
fn peaks_have_consumers(r: &Resonance) -> bool {
    r.transport.playing
        || r.transport.recording
        || r.browser.audition.playing.is_some()
        || input_monitoring_active(r)
        || meters_unsettled(r)
}

/// Drive the Reference panel's comparative loudness readout: while the
/// rail is open with at least one loaded reference, ask the engine for a
/// fresh A/B meter snapshot each tick. The reply arrives as
/// `AudioEvent::ABMeterSnapshot` and is folded into `r.reference.monitor.ab_meter`
/// by `engine_events::reference::ab_meter_snapshot`. Gated on the panel
/// being visible so we don't poll the A/B taps when nothing reads them.
fn poll_ab_meters(r: &mut Resonance) {
    if r.mixer.reference_panel_open && !r.reference.entries.is_empty() {
        let _ = r.engine.send(AudioCommand::PollABMeters);
    }
}

/// Fold a peak snapshot from the engine into the VU state. Each level
/// rises to the new peak immediately and decays only via the per-tick
/// pass in `update_vu_meters`.
pub fn apply_peak_snapshot(
    r: &mut Resonance,
    track_peaks: Vec<(TrackId, f32, f32)>,
    bus_peaks: Vec<(BusId, f32, f32)>,
    master_peak_l: f32,
    master_peak_r: f32,
) {
    for (track_id, pl, pr) in track_peaks {
        r.with_track_mut(track_id, |t| {
            if pl > t.level_l {
                t.level_l = pl;
            }
            if pr > t.level_r {
                t.level_r = pr;
            }
        });
    }
    for (bus_id, pl, pr) in bus_peaks {
        r.with_bus_mut(bus_id, |b| {
            if pl > b.level_l {
                b.level_l = pl;
            }
            if pr > b.level_r {
                b.level_r = pr;
            }
        });
    }
    if master_peak_l > r.master.level_l {
        r.master.level_l = master_peak_l;
    }
    if master_peak_r > r.master.level_r {
        r.master.level_r = master_peak_r;
    }
}

/// During playback, update the transport BPM display from the tempo
/// map. The engine computes its own BPM from the shared tempo events
/// so no `SetBpm` commands are sent here.
fn sync_tempo_at_playhead(r: &mut Resonance) {
    if !r.transport.playing || r.tempo_events.len() <= 1 && r.signature_events.len() <= 1 {
        return;
    }
    let (bpm, num, den) = r
        .tempo_map
        .tempo_at_sample(r.transport.playhead, r.sample_rate);
    // Display only — no engine command. The BPM text_input's model string
    // is only rewritten when the displayed value actually changes (the
    // `refresh_transport_labels` keyed-write idiom): iced exposes no
    // synchronous "is this field focused?" query (see `crate::focus`), so
    // an unconditional per-tick `format!` both allocated at 60 Hz and
    // clobbered any in-progress edit of the field. With the guard, an
    // edit survives every tick of a constant-tempo stretch; only a real
    // tempo change under the playhead overwrites it.
    let prev_key = bpm_display_key(r.transport.bpm);
    r.transport.bpm = bpm;
    if bpm_display_key(bpm) != prev_key {
        use std::fmt::Write;
        r.transport.bpm_input.clear();
        let _ = write!(&mut r.transport.bpm_input, "{bpm:.1}");
    }

    if num != r.transport.time_sig_num || den != r.transport.time_sig_den {
        r.transport.time_sig_num = num;
        r.transport.time_sig_den = den;
        let _ = r.engine.send(AudioCommand::SetTimeSignature {
            numerator: num,
            denominator: den,
        });
    }
}

/// Lead-in kept left of the playhead when follow pages the arrange view,
/// as a fraction of the visible width, so the playhead doesn't sit on
/// the very edge right after a page turn.
const FOLLOW_LEAD_FRACTION: f32 = 0.05;

/// Page-style playhead follow for the arrange view (review FU-D1): while
/// playing, when the playhead leaves the visible part of the outer
/// `Scrollable`, scroll so it sits just inside the left edge again. Also
/// catches a loop wrap or seek behind the view.
///
/// Follow stays out of the way of the user: it can be switched off in
/// Settings, it is paused by a manual scroll during playback
/// (`update::viewport::arrange_scrolled`) until the transport stops or
/// the playhead comes back into view (code review FU-V3b), and it is
/// skipped while a pointer drag on the timeline is in flight, so content
/// never pages away under the cursor.
fn follow_playhead(r: &mut Resonance) -> Option<Task<Message>> {
    if !r.transport.playing {
        r.viewport.follow_paused = false;
        return None;
    }
    if !r.settings.arrange.follow_playhead
        || r.view_mode != crate::state::ViewMode::Arrange
        || r.viewport.visible_width <= 0.0
        || arrange_pointer_drag_active(r)
    {
        return None;
    }
    let vp = &r.viewport;
    let playhead_x =
        (r.transport.playhead as f64 / r.sample_rate.max(1) as f64) as f32 * vp.zoom;
    let left = vp.follow_pending_x.unwrap_or(vp.scroll_offset);
    if playhead_x >= left && playhead_x < left + vp.visible_width {
        // In view. After a manual scroll that is the user and the
        // playhead meeting again: follow resumes from here.
        r.viewport.follow_paused = false;
        return None;
    }
    if vp.follow_paused {
        return None;
    }
    let max_x = (vp.scroll_content_width - vp.visible_width).max(0.0);
    let target = (playhead_x - vp.visible_width * FOLLOW_LEAD_FRACTION).clamp(0.0, max_x);
    if (target - left).abs() <= 0.5 {
        // Already as far as the content allows (playhead past the end).
        return None;
    }
    r.viewport.follow_pending_x = Some(target);
    Some(iced::widget::operation::scroll_to(
        crate::state::ARRANGE_SCROLL_ID,
        iced::widget::scrollable::AbsoluteOffset {
            x: Some(target),
            y: None,
        },
    ))
}

/// A pointer gesture on the arrange timeline is in flight (clip / MIDI
/// clip move or trim, fade or gain drag, loop-marker drag, browser drop).
fn arrange_pointer_drag_active(r: &Resonance) -> bool {
    let i = &r.interaction;
    i.clip_drag.is_some()
        || i.clip_trim.is_some()
        || i.clip_fade_drag.is_some()
        || i.clip_gain_drag.is_some()
        || i.midi_clip_drag.is_some()
        || i.midi_clip_trim.is_some()
        || r.transport.dragging_loop.is_some()
        || r.drag_placement.is_some()
}

/// Quantise a BPM to the `{:.1}` precision the transport field displays,
/// so the no-op guard compares what would actually be shown (the
/// `bpm_centi` idiom from `view::transport_labels`).
fn bpm_display_key(bpm: f32) -> u32 {
    (bpm * 10.0).round() as u32
}

impl Resonance {
    /// Test-only: the BPM text_input's model string, so the tick tests
    /// can pin that `sync_tempo_at_playhead` leaves an in-progress edit
    /// alone. Lives here (not `test_support`) because the keyed-write
    /// guard it observes is owned by this module.
    #[doc(hidden)]
    pub fn test_bpm_input(&self) -> &str {
        &self.transport.bpm_input
    }

    /// Test-only: master VU levels `(l, r)`, for the settle-floor tests.
    #[doc(hidden)]
    pub fn test_master_levels(&self) -> (f32, f32) {
        (self.master.level_l, self.master.level_r)
    }
}
