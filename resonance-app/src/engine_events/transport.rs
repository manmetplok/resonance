//! Transport / device / clock events from the engine.

use resonance_audio::midi_hardware::MidiDeviceInfo;
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
    r.error_message = Some(format!("Bounce in place failed: {e}"));
}

pub(super) fn midi_input_devices(r: &mut Resonance, devices: Vec<MidiDeviceInfo>) {
    r.midi_input_devices = devices;
}

pub(super) fn midi_output_devices(r: &mut Resonance, devices: Vec<MidiDeviceInfo>) {
    r.midi_output_devices = devices;
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
