//! A hand-rolled CLAP instrument that records every note event the host
//! hands it, for tests that drive the whole audio callback
//! ([`MixAudioHarness`](resonance_audio::test_support::MixAudioHarness))
//! and need to see what reached the plugin — stuck-note and
//! stopped-transport regressions (code review MIX-06 / MIX-08).
//!
//! Built straight from `clap_sys` vtables through
//! `__instance_from_raw_for_test`, like `multi_out_harness`. It declares
//! no audio-ports extension, so the host treats it as one stereo output.
//! While any voice is held it writes [`VOICE_LEVEL`] to its output, so a
//! test can also hear whether it ran.

#![allow(dead_code)]

use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use clap_sys::events::{
    clap_event_header, clap_event_note, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_NOTE_OFF,
    CLAP_EVENT_NOTE_ON,
};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use parking_lot::Mutex;

use resonance_audio::test_support::{PluginSlot, __instance_from_raw_for_test};

/// What the instrument writes to both channels while any voice is held.
pub const VOICE_LEVEL: f32 = 0.5;

/// One note event as the plugin received it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteEvent {
    pub on: bool,
    pub key: u8,
    /// Sample offset inside the `process()` call.
    pub time: u32,
    /// Which `process()` call (0-based) delivered it.
    pub call: usize,
}

pub struct Recorded {
    pub events: Vec<NoteEvent>,
    /// `process()` calls so far.
    pub calls: usize,
    /// Keys currently held (note-on seen, no note-off since).
    pub held: [bool; 128],
    /// The `clap_host` the plugin was created with, as an address (so the
    /// record stays `Send`): [`request_process`] calls back through it.
    pub host: usize,
    /// While set, `activate()` refuses — to leave the instance
    /// deactivated after a restart.
    pub fail_activate: bool,
}

impl Default for Recorded {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            calls: 0,
            held: [false; 128],
            host: 0,
            fail_activate: false,
        }
    }
}

impl Recorded {
    /// Whether a full all-notes-off sweep (a NoteOff for every key)
    /// arrived in `process()` call `call`.
    pub fn panicked_in(&self, call: usize) -> bool {
        let mut seen = [false; 128];
        for e in self.events.iter().filter(|e| e.call == call && !e.on) {
            seen[e.key as usize] = true;
        }
        seen.iter().all(|&s| s)
    }

    pub fn any_held(&self) -> bool {
        self.held.iter().any(|&h| h)
    }
}

pub type Recorder = Arc<Mutex<Recorded>>;

unsafe fn recorder<'a>(plugin: *const clap_plugin) -> &'a Recorder {
    &*((*plugin).plugin_data as *const Recorder)
}

unsafe extern "C" fn r_init(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn r_destroy(_plugin: *const clap_plugin) {}
unsafe extern "C" fn r_activate(p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    !recorder(p).lock().fail_activate
}
unsafe extern "C" fn r_deactivate(_plugin: *const clap_plugin) {}
unsafe extern "C" fn r_start(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn r_stop(_plugin: *const clap_plugin) {}
unsafe extern "C" fn r_reset(_plugin: *const clap_plugin) {}
unsafe extern "C" fn r_main_thread(_plugin: *const clap_plugin) {}
unsafe extern "C" fn r_get_extension(
    _plugin: *const clap_plugin,
    _id: *const std::ffi::c_char,
) -> *const c_void {
    ptr::null()
}

unsafe extern "C" fn r_process(plugin: *const clap_plugin, process: *const clap_process) -> i32 {
    let p = &*process;
    let mut rec = recorder(plugin).lock();
    let call = rec.calls;
    rec.calls += 1;

    let events = &*p.in_events;
    let count = events.size.map_or(0, |size| size(events));
    for i in 0..count {
        let Some(get) = events.get else { break };
        let header: *const clap_event_header = get(events, i);
        if header.is_null() || (*header).space_id != CLAP_CORE_EVENT_SPACE_ID {
            continue;
        }
        let ty = (*header).type_;
        if ty != CLAP_EVENT_NOTE_ON && ty != CLAP_EVENT_NOTE_OFF {
            continue;
        }
        let note = &*(header as *const clap_event_note);
        let on = ty == CLAP_EVENT_NOTE_ON;
        let key = note.key.clamp(0, 127) as u8;
        rec.held[key as usize] = on;
        rec.events.push(NoteEvent {
            on,
            key,
            time: (*header).time,
            call,
        });
    }

    let level = if rec.any_held() { VOICE_LEVEL } else { 0.0 };
    let frames = p.frames_count as usize;
    if p.audio_outputs_count >= 1 && !p.audio_outputs.is_null() {
        let out = &*p.audio_outputs;
        if !out.data32.is_null() {
            for ch in 0..(out.channel_count as usize).min(2) {
                let chan = *out.data32.add(ch);
                if chan.is_null() {
                    continue;
                }
                for f in 0..frames {
                    *chan.add(f) = level;
                }
            }
        }
    }
    1
}

/// Build the recording instrument; the returned [`Recorder`] is shared
/// with the plugin so the test can read what it received.
pub fn note_recorder(sample_rate: u32) -> (PluginSlot, Recorder) {
    let rec: Recorder = Arc::new(Mutex::new(Recorded::default()));
    let data = Box::into_raw(Box::new(Arc::clone(&rec)));
    let inst = __instance_from_raw_for_test(
        move |host| {
            // SAFETY: `data` is the live box handed to the plugin below.
            unsafe { (*data).lock().host = host as usize };
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: data as *mut c_void,
                init: Some(r_init),
                destroy: Some(r_destroy),
                activate: Some(r_activate),
                deactivate: Some(r_deactivate),
                start_processing: Some(r_start),
                stop_processing: Some(r_stop),
                reset: Some(r_reset),
                process: Some(r_process),
                get_extension: Some(r_get_extension),
                on_main_thread: Some(r_main_thread),
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        sample_rate,
    )
    .expect("note recorder builds");
    (PluginSlot::new(inst), rec)
}

/// Call `clap_host.request_process()` the way the plugin would — from
/// whatever thread the caller is on (CLAP marks it `[thread-safe]`).
pub fn request_process(rec: &Recorder) {
    let host = rec.lock().host as *const clap_sys::host::clap_host;
    assert!(!host.is_null(), "the recorder was built through the host");
    // SAFETY: the host outlives the instance the test still holds.
    unsafe {
        let request = (*host).request_process.expect("host serves request_process");
        request(host);
    }
}
