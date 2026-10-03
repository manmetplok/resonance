//! FU-D1a1: a NAM model swap that `SwapFader::try_begin_swap` cannot
//! admit — every parking slot it could fall back on is taken — is kept
//! and retried on a later block, never dropped on the audio thread.
//!
//! Needs `test-internals` (on by default for this crate's own tests via
//! the dev-dependency in `Cargo.toml`): `set_retire_sink_for_test` swaps
//! out the real janitor thread `AmpProcessor::new()` spawns for a channel
//! the test wedges full itself, so parking fills deterministically
//! instead of racing a live thread that drains almost instantly.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;

use resonance_amp::dsp::AmpProcessor;
use resonance_amp::nam::NamInference;

/// A model whose `Drop` is observable, so the test can tell a retry
/// apart from a silent drop. `id` is only for readable failure messages.
struct CountedModel {
    #[allow(dead_code)]
    id: u32,
    drops: Arc<AtomicUsize>,
}

impl CountedModel {
    fn new(id: u32, drops: &Arc<AtomicUsize>) -> Box<dyn NamInference> {
        Box::new(Self { id, drops: drops.clone() })
    }
}

impl NamInference for CountedModel {
    fn process_sample(&mut self, input: f32) -> f32 {
        input
    }
    fn reset(&mut self) {}
}

impl Drop for CountedModel {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_refused_swap_is_deferred_and_lands_once_parking_frees() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut proc = AmpProcessor::new();

    // Capacity 1, pre-filled: every later `try_send` fails (`Full` while
    // the receiver is held, then `Disconnected` once it drops —
    // `SwapFader::retire` treats both the same way), exactly like
    // resonance-dsp's own DSP2-15 test wedges a dead janitor
    // (resonance-dsp/tests/swap_fader.rs). `_rx` is kept alive for the
    // whole test, matching that test, since dropping a `Receiver` with a
    // buffered item drops the item right then — not what the "pending,
    // never dropped" assertions below are checking.
    let (wedge_tx, _rx) = sync_channel(1);
    wedge_tx.try_send(CountedModel::new(999, &drops)).expect("wedge the channel");
    proc.set_retire_sink_for_test(wedge_tx);

    // Install the first model synchronously (install-time path; no
    // crossfade, nothing displaced yet).
    proc.install_initial_model(CountedModel::new(0, &drops));

    // Every further swap displaces the previous active model into
    // retirement; the wedged channel forces it into a parking slot. Once
    // all 4 slots are occupied, the fader must refuse rather than drop.
    let mut refused_id = None;
    for id in 1..20u32 {
        proc.install_pending_model(CountedModel::new(id, &drops));
        // Run the crossfade to completion so the swap actually lands
        // (admission landed it in `pending`; `next()` via `process_block`
        // ticks it across — drive a tiny block instead of reaching into
        // the fader directly).
        let mut l = vec![0.0f32; 2048];
        let mut r = vec![0.0f32; 2048];
        proc.process_block(&mut l, &mut r, 2048);

        if proc.has_pending_swap() {
            refused_id = Some(id);
            break;
        }
    }
    let refused_id = refused_id.expect("a swap is eventually refused once parking fills");

    assert!(proc.has_pending_swap(), "the refused model must be retried, not dropped");
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "no model was dropped on the audio thread while the retry was pending"
    );

    // Nothing drains the parking slots on its own here (no owner sweep,
    // matching production — the janitor would normally do it, but this
    // test wedged it shut). Retrying while parking is still full must
    // keep refusing without dropping anything, for several blocks.
    for _ in 0..5 {
        proc.retry_pending_swap();
        assert!(proc.has_pending_swap(), "still stuck: parking was never freed");
        assert_eq!(drops.load(Ordering::SeqCst), 0);
    }

    // Free the parking slots the way a real owner would: swap in a sink
    // with room, which `can_admit`'s `flush_parked` drains them into
    // immediately (capacity alone is enough — no need for the draining
    // thread below to have run yet), then let a retire actually land.
    let (tx2, rx2) = sync_channel(8);
    proc.set_retire_sink_for_test(tx2);
    // Draining thread for the fresh channel so further retirements don't
    // wedge it too.
    std::thread::spawn(move || while rx2.recv().is_ok() {});

    let mut landed = false;
    for _ in 0..64 {
        proc.retry_pending_swap();
        let mut l = vec![0.0f32; 2048];
        let mut r = vec![0.0f32; 2048];
        proc.process_block(&mut l, &mut r, 2048);
        if !proc.has_pending_swap() {
            landed = true;
            break;
        }
    }
    assert!(landed, "deferred swap {refused_id} never landed after parking freed");
}
