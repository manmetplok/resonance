//! Transport / device / clock events from the engine.

use resonance_audio::MidiDeviceInfo;
use resonance_audio::types::*;

use crate::Resonance;

pub(super) fn stopped(r: &mut Resonance) {
    if !r.io.loading {
        r.transport.playing = false;
        r.transport.recording = false;
        r.transport.playhead = 0;
    }
}

/// The engine refused the Play/Record the app already mirrored
/// (FU-F1a): drop the optimistic flags, keep the playhead.
pub(super) fn refused(r: &mut Resonance) {
    r.transport.playing = false;
    r.transport.recording = false;
}

pub(super) fn error(r: &mut Resonance, e: String) {
    tracing::error!("Audio engine error: {}", e);
    r.error_message = Some(e);
}

pub(super) fn input_devices_listed(
    r: &mut Resonance,
    devices: Vec<InputDeviceInfo>,
    default_name: Option<String>,
) {
    r.input_devices = devices;
    r.default_input_device_name = default_name;
    // Refresh the cached `Rc<[InputDeviceInfo]>` used by the mixer
    // inspector and bounce-dialog pickers so they stop cloning the
    // full Vec every frame.
    r.view_caches.rebuild_input_devices(&r.input_devices);
}

pub(super) fn recording_started(r: &mut Resonance, start_sample: SamplePos) {
    r.transport.recording = true;
    r.transport.recording_start_sample = start_sample;
    // Each recording session is its own undo entry (STATE-02).
    r.undo.break_coalesce();
}

/// The capture ring overflowed during the active take: `dropped_frames`
/// input frames never reached the recording on disk, so the take is
/// missing audio (time-compressed / desynced from the first drop on).
/// Surfaced on the standard error banner — before this event existed
/// the take was silently corrupted with zero indication to the user.
pub(super) fn recording_overflow(r: &mut Resonance, dropped_frames: u64) {
    tracing::error!("Audio engine: recording overflow — {dropped_frames} input frames dropped");
    r.error_message = Some(format!(
        "Recording overflow: {dropped_frames} input frames were dropped — \
         this take is missing audio and may be out of sync"
    ));
}

pub(super) fn bounce_complete(r: &mut Resonance, path: String) {
    r.io.bouncing = false;
    r.io.bounce_cancel_requested = false;
    // Resolve a control-initiated `render.mixdown` job (doc #265, todo
    // #1157). The engine echoes the requested path verbatim, so the
    // token matches exactly the job that asked for this file. No-op when
    // no control job carries the token (an ordinary GUI bounce).
    let result = crate::update::control::mixdown_result(r, &path, r.sample_rate);
    r.control.jobs.complete_token(
        &crate::control_jobs::JobToken::Export {
            path: std::path::PathBuf::from(&path),
        },
        result,
    );
    tracing::info!("Bounce complete: {path}");
}

pub(super) fn bounce_error(r: &mut Resonance, e: String) {
    r.io.bouncing = false;
    // `BounceError` carries no path, but only one bounce runs at a time
    // (the render busy-guard forbids a second), so failing every live
    // control export job resolves the one in flight (todo #1157). No-op
    // when none is control-initiated.
    r.control.jobs.fail_export_jobs(e.clone());
    // A cancel the user asked for from the progress modal (FU-F1c) is
    // not a failure worth a banner.
    if !std::mem::take(&mut r.io.bounce_cancel_requested) {
        r.error_message = Some(format!("Bounce failed: {e}"));
    }
}

pub(super) fn track_bounce_error(r: &mut Resonance, e: String) {
    // Drop the in-progress modal — the run is over either way — and
    // surface the engine's reason as a banner.
    r.bounce_in_progress = None;
    r.error_message = Some(format!("Bounce in place failed: {e}"));
}

pub(super) fn track_bounce_cancelled(
    r: &mut Resonance,
    _target_track_id: resonance_audio::types::TrackId,
) {
    // Engine already removed the empty target track; just drop the
    // modal. No banner — the user explicitly cancelled.
    r.bounce_in_progress = None;
}

pub(super) fn bounce_progress(r: &mut Resonance, fraction: f32) {
    // Only one offline render runs at a time: a bounce in place, or the
    // WAV mixdown (FU-F1c).
    if let Some(state) = r.bounce_in_progress.as_mut() {
        state.fraction = fraction.clamp(0.0, 1.0);
    } else if r.io.bouncing {
        r.io.bounce_fraction = fraction.clamp(0.0, 1.0);
    }
}

/// Generalized export lifecycle (doc #196). The engine currently routes
/// the WAV path through the legacy `Bounce*` events; these mirror the
/// same UI state so the app is ready once the encoder pipeline starts
/// emitting `Export*`. `phase` is ignored for the single mix progress bar.
pub(super) fn export_progress(
    r: &mut Resonance,
    _phase: resonance_audio::types::ExportPhase,
    fraction: f32,
) {
    r.io.bouncing = true;
    r.io.bounce_fraction = fraction.clamp(0.0, 1.0);
}

pub(super) fn export_complete(r: &mut Resonance, path: String, bytes: u64) {
    r.io.bouncing = false;
    tracing::info!("Export complete: {path} ({bytes} bytes)");
}

pub(super) fn export_error(
    r: &mut Resonance,
    _kind: resonance_audio::types::ExportErrorKind,
    message: String,
) {
    r.io.bouncing = false;
    r.error_message = Some(format!("Export failed: {message}"));
}

pub(super) fn midi_input_devices(r: &mut Resonance, devices: Vec<MidiDeviceInfo>) {
    r.midi_input_devices = devices;
    r.view_caches.rebuild_midi_input(&r.midi_input_devices);
}

pub(super) fn midi_output_devices(r: &mut Resonance, devices: Vec<MidiDeviceInfo>) {
    r.midi_output_devices = devices;
    r.view_caches.rebuild_midi_output(&r.midi_output_devices);
}

pub(super) fn midi_clock_started(r: &mut Resonance) {
    r.transport.playing = true;
    r.transport.playhead = 0;
}

pub(super) fn midi_clock_continued(r: &mut Resonance) {
    r.transport.playing = true;
}

pub(super) fn midi_clock_stopped(r: &mut Resonance) {
    r.transport.playing = false;
}

pub(super) fn midi_clock_tempo_detected(r: &mut Resonance, bpm: f32) {
    r.transport.bpm = bpm;
    r.transport.bpm_input = format!("{:.1}", bpm);
}
