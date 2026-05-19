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

pub(super) fn error(r: &mut Resonance, e: String) {
    eprintln!("Audio engine error: {}", e);
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
}

pub(super) fn bounce_complete(r: &mut Resonance, path: String) {
    r.io.bouncing = false;
    eprintln!("Bounce complete: {path}");
}

pub(super) fn bounce_error(r: &mut Resonance, e: String) {
    r.io.bouncing = false;
    r.error_message = Some(format!("Bounce failed: {e}"));
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
    if let Some(state) = r.bounce_in_progress.as_mut() {
        state.fraction = fraction.clamp(0.0, 1.0);
    }
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
