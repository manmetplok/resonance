//! State-machine tests for `SwapFader`: idle, direct install, fade-in
//! from empty, the fade-out -> swap -> fade-in crossfade, pending
//! replacement mid-fade, and the retirement mechanism that keeps
//! displaced payloads from being dropped on the audio thread.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_dsp::SwapFader;

/// Payload that counts its drops, to prove the fader ships displaced
/// payloads out through the retirement sink instead of running `Drop`
/// in place.
struct DropCounted {
    id: u32,
    drops: Arc<AtomicUsize>,
}

impl DropCounted {
    fn new(id: u32, drops: &Arc<AtomicUsize>) -> Self {
        Self {
            id,
            drops: drops.clone(),
        }
    }
}

impl Drop for DropCounted {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn idle_with_no_payload_is_unity_gain() {
    let mut fader: SwapFader<u32> = SwapFader::new(4);
    assert!(fader.active().is_none());
    assert!(!fader.is_fading_out());
    for _ in 0..8 {
        let (gain, payload) = fader.next();
        assert_eq!(gain, 1.0);
        assert!(payload.is_none());
    }
}

#[test]
fn install_is_immediate_with_no_fade() {
    let mut fader = SwapFader::new(4);
    fader.install(7u32);
    assert_eq!(fader.active(), Some(&7));
    let (gain, payload) = fader.next();
    assert_eq!(gain, 1.0);
    assert_eq!(payload.copied(), Some(7));
}

#[test]
fn swap_into_empty_fades_in_immediately() {
    let mut fader = SwapFader::new(4);
    fader.begin_swap(1u32);
    assert_eq!(fader.active(), Some(&1));
    assert!(!fader.is_fading_out());
    let gains: Vec<f32> = (0..6).map(|_| fader.next().0).collect();
    assert_eq!(gains, vec![0.25, 0.5, 0.75, 1.0, 1.0, 1.0]);
}

#[test]
fn swap_over_active_crossfades_out_then_in() {
    let mut fader = SwapFader::new(4);
    fader.install(1u32);
    fader.begin_swap(2u32);
    assert!(fader.is_fading_out());

    let log: Vec<(f32, u32)> = (0..9)
        .map(|_| {
            let (gain, payload) = fader.next();
            (gain, payload.copied().unwrap())
        })
        .collect();
    assert_eq!(
        log,
        vec![
            // Fade-out runs on the old payload...
            (0.75, 1),
            (0.5, 1),
            (0.25, 1),
            // ...the swap lands on the silent sample...
            (0.0, 2),
            // ...and the new payload fades in.
            (0.25, 2),
            (0.5, 2),
            (0.75, 2),
            (1.0, 2),
            (1.0, 2),
        ]
    );
}

#[test]
fn second_swap_mid_fade_replaces_pending_and_continues_fade_out() {
    let mut fader = SwapFader::new(4);
    fader.install(1u32);
    fader.begin_swap(2u32);
    fader.next();
    fader.next();

    // A newer payload arrives before the swap lands: it supersedes the
    // pending one and the fade-out carries on from where it was (DSP-07
    // — restarting it at full gain was a step discontinuity).
    fader.begin_swap(3u32);
    let log: Vec<(f32, u32)> = (0..5)
        .map(|_| {
            let (gain, payload) = fader.next();
            (gain, payload.copied().unwrap())
        })
        .collect();
    assert_eq!(
        log,
        vec![(0.25, 1), (0.0, 3), (0.25, 3), (0.5, 3), (0.75, 3)]
    );
}

#[test]
fn swap_mid_fade_in_fades_out_from_the_current_gain() {
    let mut fader = SwapFader::new(4);
    fader.install(1u32);
    fader.begin_swap(2u32);
    // 0.75, 0.5, 0.25, 0.0 (lands on 2), then fade-in 0.25, 0.5.
    for _ in 0..6 {
        fader.next();
    }
    fader.begin_swap(3u32);
    let log: Vec<(f32, u32)> = (0..5)
        .map(|_| {
            let (gain, payload) = fader.next();
            (gain, payload.copied().unwrap())
        })
        .collect();
    assert_eq!(
        log,
        vec![(0.25, 2), (0.0, 3), (0.25, 3), (0.5, 3), (0.75, 3)]
    );
}

/// DSP-07: under continuous retargeting (granular Fade mode with Time
/// automated calls `begin_swap` every block) the gain must never step by
/// more than one fade step, and the latest target must eventually land.
#[test]
fn repeated_begin_swap_keeps_gain_continuous_and_lands() {
    const FADE: u32 = 256;
    let step = 1.0 / FADE as f32;
    let mut fader = SwapFader::new(FADE);
    fader.install(0u32);
    let mut prev = 1.0_f32;
    let mut target = 0u32;
    let mut landed_values = Vec::new();
    for n in 0..4096u32 {
        if n % 64 == 0 {
            target += 1;
            fader.begin_swap(target);
        }
        let (gain, payload) = fader.next();
        let v = *payload.unwrap();
        assert!(
            (gain - prev).abs() <= step + 1e-6,
            "gain jumped {prev} -> {gain} at sample {n}"
        );
        prev = gain;
        if landed_values.last() != Some(&v) {
            landed_values.push(v);
        }
    }
    // Swaps must actually land while the retargeting continues — the
    // tap moved at all — and never to a superseded value's predecessor.
    assert!(landed_values.len() > 3, "swap never landed: {landed_values:?}");
    assert!(landed_values.windows(2).all(|w| w[1] > w[0]));
    // Once retargeting stops, the most recent target lands and the gain
    // returns to unity.
    for _ in 0..(2 * FADE) {
        fader.next();
    }
    let (gain, payload) = fader.next();
    assert_eq!((gain, *payload.unwrap()), (1.0, target));
}

#[test]
fn swap_land_retires_old_payload_instead_of_dropping() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = sync_channel(4);
    let mut fader = SwapFader::new(4);
    fader.set_retire_sink(tx);

    fader.install(DropCounted::new(1, &drops));
    fader.begin_swap(DropCounted::new(2, &drops));
    for _ in 0..4 {
        fader.next();
    }

    // The swap has landed: the old payload came out via the sink, with
    // its destructor never run inside `next()`.
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let retired = rx.try_recv().expect("old payload retired via sink");
    assert_eq!(retired.id, 1);
    assert_eq!(fader.active().map(|p| p.id), Some(2));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(retired);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn rapid_double_swap_retires_superseded_pending() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = sync_channel(4);
    let mut fader = SwapFader::new(4);
    fader.set_retire_sink(tx);

    fader.install(DropCounted::new(1, &drops));
    fader.begin_swap(DropCounted::new(2, &drops));
    fader.next();
    fader.next();

    // A newer payload arrives before the swap lands: the superseded
    // pending payload is retired by `begin_swap`, not dropped there.
    fader.begin_swap(DropCounted::new(3, &drops));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(rx.try_recv().expect("superseded pending retired").id, 2);

    for _ in 0..4 {
        fader.next();
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1); // payload 2, receiver-side
    assert_eq!(rx.try_recv().expect("faded-out active retired").id, 1);
    assert_eq!(fader.active().map(|p| p.id), Some(3));
}

#[test]
fn install_retires_displaced_active_and_pending() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = sync_channel(4);
    let mut fader = SwapFader::new(4);
    fader.set_retire_sink(tx);

    fader.install(DropCounted::new(1, &drops));
    fader.begin_swap(DropCounted::new(2, &drops));
    fader.install(DropCounted::new(3, &drops));

    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(rx.try_recv().expect("displaced active retired").id, 1);
    assert_eq!(rx.try_recv().expect("displaced pending retired").id, 2);
    assert_eq!(fader.active().map(|p| p.id), Some(3));
}

#[test]
fn full_channel_parks_payload_instead_of_dropping() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = sync_channel(1);
    let mut fader = SwapFader::new(4);
    fader.set_retire_sink(tx.clone());

    // Wedge the channel so the fader's try_send has nowhere to go.
    tx.try_send(DropCounted::new(99, &drops))
        .expect("pre-fill the single-slot channel");

    fader.install(DropCounted::new(1, &drops));
    fader.begin_swap(DropCounted::new(2, &drops));
    // Rapid re-selection with a full channel: the superseded pending
    // payload is parked inline, never dropped on the caller's thread.
    fader.begin_swap(DropCounted::new(3, &drops));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let parked = fader.take_retired().expect("payload parked on full channel");
    assert_eq!(parked.id, 2);
    assert!(fader.take_retired().is_none());

    // Once the channel drains, the next retirement goes through again.
    assert_eq!(rx.try_recv().expect("pre-fill payload").id, 99);
    for _ in 0..4 {
        fader.next();
    }
    assert_eq!(rx.try_recv().expect("faded-out active retired").id, 1);
}

#[test]
fn janitor_thread_drops_retired_payloads_off_caller() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut fader = SwapFader::new(4);
    fader.set_retire_sink(SwapFader::spawn_retire_janitor("swap-fader-test-janitor"));

    fader.install(DropCounted::new(1, &drops));
    fader.begin_swap(DropCounted::new(2, &drops));
    for _ in 0..4 {
        fader.next();
    }

    // The janitor receives and drops the retired payload on its own
    // thread; wait (bounded) for that drop to land.
    let deadline = Instant::now() + Duration::from_secs(5);
    while drops.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(fader.active().map(|p| p.id), Some(2));
}

#[test]
fn without_sink_displaced_payloads_still_drop_in_place() {
    // The pre-retirement contract, kept for payloads with trivial
    // destructors (e.g. `SwapFader<f32>` in resonance-granular-delay):
    // no sink means displaced payloads drop where they always did.
    let drops = Arc::new(AtomicUsize::new(0));
    let mut fader = SwapFader::new(4);

    fader.install(DropCounted::new(1, &drops));
    fader.begin_swap(DropCounted::new(2, &drops));
    for _ in 0..4 {
        fader.next();
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn crossfade_sequence_is_unchanged_with_retirement_active() {
    // Same script as `swap_over_active_crossfades_out_then_in`, with a
    // retirement sink attached: the fade envelope and swap timing must
    // be bit-identical to the drop-in-place fader's.
    let drops = Arc::new(AtomicUsize::new(0));
    let (tx, _rx) = sync_channel(4);
    let mut fader = SwapFader::new(4);
    fader.set_retire_sink(tx);

    fader.install(DropCounted::new(1, &drops));
    fader.begin_swap(DropCounted::new(2, &drops));
    assert!(fader.is_fading_out());

    let log: Vec<(f32, u32)> = (0..9)
        .map(|_| {
            let (gain, payload) = fader.next();
            (gain, payload.expect("payload present throughout").id)
        })
        .collect();
    assert_eq!(
        log,
        vec![
            (0.75, 1),
            (0.5, 1),
            (0.25, 1),
            (0.0, 2),
            (0.25, 2),
            (0.5, 2),
            (0.75, 2),
            (1.0, 2),
            (1.0, 2),
        ]
    );
}
