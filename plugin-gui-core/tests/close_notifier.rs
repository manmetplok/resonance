//! `CloseNotifier`: the runtime's "window closed itself" signal to the host
//! (PLG-01). One-shot, silent after a host-initiated teardown, and not lost
//! when the window closes before the callback is installed.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use plugin_gui_core::CloseNotifier;

fn counter(n: &Arc<AtomicU32>) -> impl FnOnce() + Send + 'static {
    let n = Arc::clone(n);
    move || {
        n.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_self_close_runs_the_callback_exactly_once() {
    let calls = Arc::new(AtomicU32::new(0));
    let notifier = CloseNotifier::new();
    notifier.set_callback(counter(&calls));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let runtime_side = notifier.clone();
    std::thread::spawn(move || runtime_side.notify())
        .join()
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(notifier.has_fired());

    notifier.notify();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "notify is one-shot");
}

#[test]
fn a_close_before_the_callback_is_installed_is_not_lost() {
    let calls = Arc::new(AtomicU32::new(0));
    let notifier = CloseNotifier::new();
    notifier.notify();
    notifier.set_callback(counter(&calls));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_host_initiated_teardown_is_never_reported() {
    let calls = Arc::new(AtomicU32::new(0));
    let notifier = CloseNotifier::new();
    notifier.set_callback(counter(&calls));
    notifier.disarm();
    // The runtime's thread exits after the host's Quit and notifies on
    // its way out, exactly as it does after a user close.
    notifier.notify();
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    // Nor does a callback installed after the teardown run.
    notifier.set_callback(counter(&calls));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
