//! The insert-chain probe (warmth-width-depth.md §7.3): drive a CLONE of
//! one insert chain with a synthetic tone and read its harmonic
//! signature.
//!
//! ## Why a clone, and what it is
//!
//! The offline renderers process the LIVE plugin instances (under
//! `OfflineRenderGuard`), which advances their state — compressor gain
//! reduction, envelopes, a saturator's filters, the mastering
//! assistant's capture — and mutes live output while they run. A probe
//! must not do that: it is a question about a chain, asked mid-mix.
//!
//! So [`handle_probe_chain`] builds a fresh offline chain on the engine
//! thread (the only thread that owns the loaded bundles):
//!
//! 1. For every stage, lock the live instance just long enough for one
//!    `save_state()` — the same read a project save does. If a live
//!    instance is locked by a render block, the whole command is
//!    re-enqueued for the next engine tick instead of waiting.
//! 2. Create a NEW instance of the same plugin from the same bundle
//!    (`create_instance`, activated at the engine rate) and load that
//!    state into it (`reload_with_state`). A plugin without the CLAP
//!    state extension is probed at its defaults and reported with
//!    `state_copied: false`.
//! 3. Move the clones to a worker thread, run the stimulus through them
//!    in chain order and analyze; then hand them back to the engine
//!    thread, which destroys them (CLAP `deactivate` / `destroy` are
//!    main-thread calls, and the engine thread is this host's main
//!    thread for plugins).
//!
//! The live instances are never processed, reset or reloaded, so neither
//! playback, automation, undo nor any plugin's running state is touched,
//! and no offline-render guard is taken: a probe can run while the
//! transport rolls or a bounce renders. Automation is not applied to the
//! clone — it probes the chain at its current parameter values — and a
//! sidechain key input receives silence.

use crate::clap_host::SyncClapInstance;
use crate::types::*;
use resonance_metering::probe::{
    analyze_harmonics, bin_exact_hz, imd_pct, probe_sine, smpte_pair, PROBE_LEN,
};

use super::internal::{EngineInternal, ProbeClones};
use super::plugins::{ensure_bundle, resolve_plugin_id};
use super::thread::{HandlerCtx, HandlerState};

/// Block size the clones are driven with.
const PROBE_BLOCK: usize = 1_024;

/// Clone the chain `stages` names and probe it on a worker thread; emits
/// exactly one terminal event echoing `probe_id` (or re-enqueues itself
/// when a live instance is momentarily locked).
pub(crate) fn handle_probe_chain(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    probe_id: u64,
    stages: Vec<ProbeStage>,
    spec: ProbeSpec,
) {
    let fail = |message: String| {
        let _ = ctx.event_tx.send(AudioEvent::ChainProbeError { probe_id, message });
    };

    // 1. Read every live state first, so a contended lock retries before
    //    anything has been built.
    let plugins = ctx.plugins();
    let mut saved = Vec::with_capacity(stages.len());
    for stage in &stages {
        let Some(slot) = plugins.get(&stage.instance_id) else {
            fail(format!("plugin instance {} is not loaded", stage.instance_id));
            return;
        };
        let Some(live) = slot.try_lock() else {
            let _ = ctx.cmd_tx_retry.send(AudioCommand::ProbeChain {
                probe_id,
                stages,
                spec,
            });
            return;
        };
        saved.push(live.0.save_state());
    }

    // 2. Fresh instances of the same plugins, carrying that state.
    let mut chain = Vec::with_capacity(stages.len());
    let mut probed = Vec::with_capacity(stages.len());
    for (stage, state_bytes) in stages.iter().zip(saved) {
        let path = std::path::Path::new(&stage.clap_file_path);
        let built = ensure_bundle(&mut state.bundles, path, &stage.clap_plugin_id)
            .and_then(|idx| {
                resolve_plugin_id(&state.bundles[idx], stage.clap_plugin_id.clone())
                    .map(|id| (idx, id))
            })
            .map_err(|e| e.to_string())
            .and_then(|(idx, id)| {
                state.bundles[idx]
                    .create_instance(&id, ctx.sample_rate)
                    .map_err(|e| e.to_string())
            });
        let mut instance = match built {
            Ok(instance) => instance,
            Err(e) => {
                fail(format!("could not clone {} for the probe: {e}", stage.clap_plugin_id));
                return;
            }
        };
        let state_copied = state_bytes.is_some_and(|bytes| instance.reload_with_state(&bytes));
        probed.push(ProbedStage {
            instance_id: stage.instance_id,
            state_copied,
        });
        chain.push(SyncClapInstance(instance));
    }

    // 3. Render and analyze off the engine thread.
    spawn_probe(ctx, probe_id, chain, probed, spec);
}

/// Probe `chain` on a `probe-chain` worker and emit the one terminal
/// event; the clones then go back to the engine thread to be destroyed.
///
/// CLAP makes `deactivate` and `destroy` main-thread calls, and this
/// host's main thread for plugins is the engine thread (every instance is
/// created, activated and destroyed there). So the worker never drops a
/// clone: once the report is out — or the run panicked — it posts them on
/// the engine inbox ([`EngineInternal::RetireProbeClones`]), whose
/// handler drops them on the engine thread, as the retire sweep does for
/// a removed live slot.
pub(crate) fn spawn_probe(
    ctx: &HandlerCtx,
    probe_id: u64,
    chain: Vec<SyncClapInstance>,
    probed: Vec<ProbedStage>,
    spec: ProbeSpec,
) {
    let event_tx = ctx.event_tx.clone();
    let shared = std::sync::Arc::clone(ctx.shared);
    let sample_rate = ctx.sample_rate;
    std::thread::Builder::new()
        .name("probe-chain".into())
        .spawn(move || {
            let mut chain = chain;
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "probe-chain",
                || {
                    let report = run_probe(&mut chain, probed, spec, sample_rate);
                    let _ = event_tx.send(AudioEvent::ChainProbed { probe_id, report });
                },
                |message| {
                    let _ = panic_tx.send(AudioEvent::ChainProbeError { probe_id, message });
                },
            );
            shared
                .inbox
                .post(EngineInternal::RetireProbeClones(ProbeClones(chain)));
        })
        .expect("spawn probe-chain thread");
}

/// Drive `chain` (already-built instances, in order) with the probe tone
/// and, if asked, the SMPTE pair, and analyze the steady-state output.
///
/// Pure over its instances: nothing outside them is read or written, and
/// they stay the caller's to destroy (on the engine thread — see
/// [`spawn_probe`]).
pub fn run_probe(
    chain: &mut [SyncClapInstance],
    stages: Vec<ProbedStage>,
    spec: ProbeSpec,
    sample_rate: u32,
) -> ChainProbeReport {
    let rate = f64::from(sample_rate);
    let latency_samples: u32 = chain.iter().map(|i| i.0.latency_samples()).sum();
    // Settle: at least half a second, and a quarter-second past the
    // chain's latency, so filters, envelopes and lookahead are steady.
    let rate_frames = sample_rate as usize;
    let warmup = (rate_frames / 2).max(latency_samples as usize + rate_frames / 4);
    let frames = (warmup + PROBE_LEN).div_ceil(PROBE_BLOCK) * PROBE_BLOCK;

    let freq = bin_exact_hz(rate, spec.freq_hz);
    let tone = probe_sine(rate, freq, spec.level_dbfs, frames);
    let out = process(chain, &tone);
    let harmonics = analyze_harmonics(rate, freq, &out[warmup..warmup + PROBE_LEN]);

    let imd = spec.imd.then(|| {
        for instance in chain.iter_mut() {
            instance.0.reset_processing();
            instance.0.reset();
        }
        let (pair, low, high) = smpte_pair(rate, spec.level_dbfs, frames);
        let out = process(chain, &pair);
        imd_pct(rate, low, high, &out[warmup..warmup + PROBE_LEN])
    });

    ChainProbeReport {
        stages,
        harmonics,
        imd_pct: imd.flatten(),
        latency_samples,
    }
}

/// Run `input` (mono, fed to both channels) through `chain` in
/// [`PROBE_BLOCK`] blocks; returns the left output.
fn process(chain: &mut [SyncClapInstance], input: &[f32]) -> Vec<f32> {
    // Process calls are `[audio-thread]` in CLAP; the offline renderers
    // enter the same scope (`bounce::render`).
    let _audio = crate::clap_host::thread_check::AudioThreadScope::enter();
    let mut out = Vec::with_capacity(input.len());
    let mut left = vec![0.0f32; PROBE_BLOCK];
    let mut right = vec![0.0f32; PROBE_BLOCK];
    for block in input.chunks(PROBE_BLOCK) {
        let n = block.len();
        left[..n].copy_from_slice(block);
        right[..n].copy_from_slice(block);
        for instance in chain.iter_mut() {
            instance.0.process(&mut left[..n], &mut right[..n], n);
        }
        out.extend_from_slice(&left[..n]);
    }
    out
}
