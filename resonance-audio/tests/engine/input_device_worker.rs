//! Input enumeration never runs on the engine control thread
//! (`engine::input_devices`).
//!
//! On macOS, enumerating inputs blocks until the microphone-permission
//! prompt is answered. The app asks for the list at startup, and while
//! that ran inline the engine served nothing else: the CLAP scan never
//! reported, the plugin catalog stayed empty, and no plugin could be
//! added. These tests stand in for the prompt with an enumerator that
//! blocks until a test opens it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioCommand, AudioEvent, InputDeviceInfo};
use resonance_common::ExternalInstrument;

const TRACK: u64 = 7;

/// How long a result may take to show up once nothing blocks it.
const ARRIVAL: Duration = Duration::from_secs(5);

/// A latch the enumerator waits on — the unanswered prompt.
#[derive(Default)]
struct Prompt {
    answered: Mutex<bool>,
    cv: Condvar,
}

impl Prompt {
    fn wait(&self) {
        let mut answered = self.answered.lock().unwrap();
        while !*answered {
            answered = self.cv.wait(answered).unwrap();
        }
    }

    fn answer(&self) {
        *self.answered.lock().unwrap() = true;
        self.cv.notify_all();
    }
}

fn device(name: &str) -> InputDeviceInfo {
    InputDeviceInfo {
        name: name.to_string(),
        description: name.to_string(),
        channels: 2,
    }
}

/// An enumerator that blocks on `prompt` and counts its calls.
fn blocking(
    prompt: &Arc<Prompt>,
    calls: &Arc<AtomicUsize>,
) -> impl Fn() -> (Vec<InputDeviceInfo>, Option<String>) + Send + Sync + 'static {
    let (prompt, calls) = (Arc::clone(prompt), Arc::clone(calls));
    move || {
        calls.fetch_add(1, Ordering::SeqCst);
        prompt.wait();
        (vec![device("Built-in Mic")], Some("Built-in Mic".to_string()))
    }
}

/// Every `InputDevicesListed` the harness has emitted so far.
fn listed(h: &mut EngineHandlerHarness) -> Vec<Vec<String>> {
    h.drain_events()
        .into_iter()
        .filter_map(|e| match e {
            AudioEvent::InputDevicesListed { devices, .. } => {
                Some(devices.into_iter().map(|d| d.name).collect())
            }
            _ => None,
        })
        .collect()
}

/// Poll `until` for up to [`ARRIVAL`].
fn eventually(mut until: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + ARRIVAL;
    while Instant::now() < deadline {
        if until() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    until()
}

#[test]
fn listing_inputs_returns_at_once_while_enumeration_blocks() {
    let mut h = EngineHandlerHarness::new();
    let (prompt, calls) = (Arc::new(Prompt::default()), Arc::new(AtomicUsize::new(0)));
    h.set_input_enumerator(blocking(&prompt, &calls));

    let started = Instant::now();
    h.dispatch(AudioCommand::ListInputDevices);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "ListInputDevices held the engine thread for {:?}",
        started.elapsed()
    );
    assert!(eventually(|| calls.load(Ordering::SeqCst) == 1), "the worker never enumerated");

    // The engine keeps serving commands while the enumeration waits.
    let events = h.add_track(TRACK, Some("Vocal".to_string()));
    assert!(!events.is_empty(), "adding a track produced nothing: {events:?}");
    assert!(listed(&mut h).is_empty(), "a list arrived before the prompt was answered");

    prompt.answer();
    let mut got = Vec::new();
    assert!(eventually(|| {
        got.extend(listed(&mut h));
        !got.is_empty()
    }));
    assert_eq!(got, vec![vec!["Built-in Mic".to_string()]]);
}

#[test]
fn requests_during_a_blocked_enumeration_share_one_more_pass() {
    let mut h = EngineHandlerHarness::new();
    let (prompt, calls) = (Arc::new(Prompt::default()), Arc::new(AtomicUsize::new(0)));
    h.set_input_enumerator(blocking(&prompt, &calls));

    h.dispatch(AudioCommand::ListInputDevices);
    assert!(eventually(|| calls.load(Ordering::SeqCst) == 1));
    // Queued behind the stuck pass: no new thread, no new enumeration yet.
    for _ in 0..3 {
        h.dispatch(AudioCommand::ListInputDevices);
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a queued request started its own enumeration");

    prompt.answer();
    let mut got = Vec::new();
    assert!(eventually(|| {
        got.extend(listed(&mut h));
        got.len() >= 2
    }));
    std::thread::sleep(Duration::from_millis(50));
    got.extend(listed(&mut h));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "the queued requests share one pass");
    assert_eq!(got.len(), 2, "one list per pass: {got:?}");
}

#[test]
fn a_device_check_reports_a_missing_return_input_after_the_worker_answers() {
    let mut h = EngineHandlerHarness::new();
    h.add_track(TRACK, Some("Synth".to_string()));
    h.dispatch(AudioCommand::SetExternalInstrument {
        config: ExternalInstrument::new(TRACK),
    });
    h.dispatch(AudioCommand::SetTrackInputDevice {
        track_id: TRACK,
        device_name: Some("USB Interface".to_string()),
    });
    h.set_input_enumerator(|| (vec![device("Built-in Mic")], None));
    h.drain_events();

    h.dispatch(AudioCommand::CheckExternalInstrumentDevices { track_id: TRACK });
    let mut offline = Vec::new();
    assert!(
        eventually(|| {
            h.apply_worker_results();
            offline.extend(h.drain_events().into_iter().filter_map(|e| match e {
                AudioEvent::ExternalInstrumentReturnInputOffline { track_id, device } => {
                    Some((track_id, device))
                }
                _ => None,
            }));
            !offline.is_empty()
        }),
        "the missing return input was never reported"
    );
    assert_eq!(offline, vec![(TRACK, Some("USB Interface".to_string()))]);
}

#[test]
fn a_panicking_enumeration_does_not_stop_later_requests() {
    let mut h = EngineHandlerHarness::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    h.set_input_enumerator(move || {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("driver fell over");
        }
        (vec![device("Built-in Mic")], None)
    });

    h.dispatch(AudioCommand::ListInputDevices);
    let mut got = Vec::new();
    assert!(eventually(|| {
        got.extend(listed(&mut h));
        !got.is_empty()
    }));
    assert_eq!(got, vec![Vec::<String>::new()], "a failed pass reports no inputs");

    h.dispatch(AudioCommand::ListInputDevices);
    let mut again = Vec::new();
    assert!(
        eventually(|| {
            again.extend(listed(&mut h));
            !again.is_empty()
        }),
        "the worker stopped serving after a panic"
    );
    assert_eq!(again, vec![vec!["Built-in Mic".to_string()]]);
}
