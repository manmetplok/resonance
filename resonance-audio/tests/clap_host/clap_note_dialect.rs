//! Note dialect negotiation and MIDI controllers (code review HOST-13).
//!
//! The host sent `CLAP_EVENT_NOTE_ON` / `NOTE_OFF` to every plugin and
//! nothing else, without reading `clap.note-ports`: a MIDI-dialect-only
//! instrument heard nothing, and mod wheel, pitch bend and aftertouch
//! never reached any instrument. Now the note port decides — notes go as
//! MIDI to a port that prefers (or only takes) MIDI, and live controllers
//! go as `CLAP_EVENT_MIDI` to any port that takes MIDI — all merged into
//! the block's one time-sorted event list with the param changes.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap_sys::events::{
    clap_event_midi, clap_event_note, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON,
    CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::note_ports::{
    clap_note_port_info, clap_plugin_note_ports, CLAP_EXT_NOTE_PORTS, CLAP_NOTE_DIALECT_CLAP,
    CLAP_NOTE_DIALECT_MIDI,
};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{
    ClapInstance, LiveMidiEvent, MixAudioHarness, PluginSlot, __instance_from_raw_for_test,
};
use resonance_audio::types::*;

/// What the fake saw, per event: `(time, kind, bytes)` where kind is
/// `'n'` / `'f'` (CLAP note on / off, bytes = key), `'m'` (MIDI, bytes =
/// data) or `'p'` (param value).
type Seen = Arc<Mutex<Vec<(u32, char, [u8; 3])>>>;

struct Fake {
    supported: u32,
    preferred: u32,
    seen: Seen,
}

unsafe fn fake<'a>(p: *const clap_plugin) -> &'a Fake {
    unsafe { &*((*p).plugin_data as *const Fake) }
}

unsafe extern "C" fn ok(_p: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn noop(_p: *const clap_plugin) {}
unsafe extern "C" fn activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}

unsafe extern "C" fn ports_count(_p: *const clap_plugin, is_input: bool) -> u32 {
    u32::from(is_input)
}

unsafe extern "C" fn ports_get(
    p: *const clap_plugin,
    _index: u32,
    _is_input: bool,
    info: *mut clap_note_port_info,
) -> bool {
    let f = unsafe { fake(p) };
    let info = unsafe { &mut *info };
    info.id = 0;
    info.supported_dialects = f.supported;
    info.preferred_dialect = f.preferred;
    true
}

static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(ports_count),
    get: Some(ports_get),
};

unsafe extern "C" fn get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_EXT_NOTE_PORTS {
        &NOTE_PORTS as *const clap_plugin_note_ports as *const c_void
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn process(p: *const clap_plugin, process: *const clap_process) -> clap_process_status {
    unsafe {
        let f = fake(p);
        let events = &*(*process).in_events;
        let mut seen = f.seen.lock().unwrap();
        for i in 0..(events.size.unwrap())(events) {
            let h = &*(events.get.unwrap())(events, i);
            match h.type_ {
                CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF => {
                    let e = &*(h as *const _ as *const clap_event_note);
                    let kind = if h.type_ == CLAP_EVENT_NOTE_ON { 'n' } else { 'f' };
                    seen.push((h.time, kind, [e.key as u8, 0, 0]));
                }
                CLAP_EVENT_MIDI => {
                    let e = &*(h as *const _ as *const clap_event_midi);
                    seen.push((h.time, 'm', e.data));
                }
                CLAP_EVENT_PARAM_VALUE => seen.push((h.time, 'p', [0; 3])),
                _ => {}
            }
        }
    }
    CLAP_PROCESS_CONTINUE
}

fn instrument(supported: u32, preferred: u32) -> (ClapInstance, Seen) {
    let seen: Seen = Arc::default();
    let data = Box::into_raw(Box::new(Fake {
        supported,
        preferred,
        seen: seen.clone(),
    }));
    let inst = __instance_from_raw_for_test(
        |_host| {
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: data as *mut c_void,
                init: Some(ok),
                destroy: Some(noop),
                activate: Some(activate),
                deactivate: Some(noop),
                start_processing: Some(ok),
                stop_processing: Some(noop),
                reset: Some(noop),
                process: Some(process),
                get_extension: Some(get_extension),
                on_main_thread: None,
            })) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake instrument");
    (inst, seen)
}

fn run(inst: &mut ClapInstance) {
    let mut l = vec![0.0f32; 64];
    let mut r = vec![0.0f32; 64];
    inst.process(&mut l, &mut r, 64);
}

#[test]
fn a_midi_only_instrument_gets_its_notes_as_midi() {
    let (mut inst, seen) = instrument(CLAP_NOTE_DIALECT_MIDI, CLAP_NOTE_DIALECT_MIDI);
    assert!(inst.accepts_midi());
    inst.queue_note_on(60, 1.0, 5);
    inst.queue_note_off(60, 20);
    run(&mut inst);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(5, 'm', [0x90, 60, 127]), (20, 'm', [0x80, 60, 0])],
        "notes must reach a MIDI-only port as MIDI, and only as MIDI"
    );
}

#[test]
fn controllers_reach_a_port_that_takes_midi_merged_by_time() {
    // The first-party bridge's port: CLAP preferred, MIDI accepted.
    let (mut inst, seen) = instrument(
        CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI,
        CLAP_NOTE_DIALECT_CLAP,
    );
    inst.queue_midi([0xE0, 0, 0x50], 30);
    inst.queue_note_on(64, 0.5, 10);
    inst.queue_midi([0xB0, 1, 99], 10);
    inst.queue_param_at(7, 0.25, 10);
    inst.queue_param_at(7, 0.5, 40);
    run(&mut inst);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            // At equal times: param, then note, then MIDI.
            (10, 'p', [0; 3]),
            (10, 'n', [64, 0, 0]),
            (10, 'm', [0xB0, 1, 99]),
            (30, 'm', [0xE0, 0, 0x50]),
            (40, 'p', [0; 3]),
        ],
        "notes stay CLAP on a CLAP-preferring port; one time-sorted list"
    );
}

#[test]
fn a_clap_only_instrument_gets_no_midi() {
    let (mut inst, seen) = instrument(CLAP_NOTE_DIALECT_CLAP, CLAP_NOTE_DIALECT_CLAP);
    assert!(!inst.accepts_midi());
    inst.queue_midi([0xB0, 1, 99], 0);
    inst.queue_note_on(60, 1.0, 0);
    run(&mut inst);
    assert_eq!(*seen.lock().unwrap(), vec![(0, 'n', [60, 0, 0])]);
}

/// The whole live path: controller bytes from a track's MIDI input reach
/// its instrument through the audio callback, with the transport stopped.
#[test]
fn a_live_mod_wheel_reaches_the_tracks_instrument() {
    const TRACK: TrackId = 1;
    const INSTRUMENT: PluginInstanceId = 300;
    let (inst, seen) = instrument(CLAP_NOTE_DIALECT_MIDI, CLAP_NOTE_DIALECT_MIDI);
    let track = Track::with_type(TRACK, "Synth".into(), TrackType::Instrument);
    track.push_plugin(INSTRUMENT);
    let mut h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        128,
        2,
        48_000,
        true,
    );
    h.edit_plugins(|p| p.insert(INSTRUMENT, Arc::new(PluginSlot::new(inst))));
    assert!(!h.shared().playing.load(Ordering::Relaxed));
    h.send_live_midi(LiveMidiEvent::InboundMidi {
        track_id: TRACK,
        data: [0xB0, 1, 100],
        arrival: Instant::now(),
    });
    h.send_live_midi(LiveMidiEvent::InboundNoteOn {
        track_id: TRACK,
        note: 62,
        velocity: 1.0,
        arrival: Instant::now(),
    });
    h.render();
    let seen = seen.lock().unwrap();
    let kinds: Vec<[u8; 3]> = seen.iter().filter(|e| e.1 == 'm').map(|e| e.2).collect();
    assert!(kinds.contains(&[0xB0, 1, 100]), "mod wheel delivered: {seen:?}");
    assert!(kinds.contains(&[0x90, 62, 127]), "note delivered as MIDI: {seen:?}");
}
