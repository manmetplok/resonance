//! DSP-16: the chain's ten FIR convolvers (two EQs and three crossovers,
//! two channels each) used to run their FFT iterations in phase — all
//! ten in the same host callback once per hop, idle in between. The
//! worst callback, not the average, decides xruns, so the chain now
//! staggers their hop phases: no host quantum up to [`QUANTUM`] frames
//! holds two iterations. Latency is unchanged (pinned by the golden and
//! the latency tests).

use resonance_mastering::chain::Chain;
use resonance_mastering::stages::linear_phase_eq::FirGeometry;
use resonance_mastering::viz::MasteringViz;

/// The PipeWire quantum the project pins (48 kHz / 128).
const QUANTUM: usize = 128;

#[test]
fn convolver_iterations_fall_in_different_callbacks() {
    for sr in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let viz = MasteringViz::new();
        let chain = Chain::new(sr, 512, &viz);
        let hop = FirGeometry::for_sample_rate(sr).hop;
        let countdowns = chain.convolver_iteration_countdowns();
        for (i, &a) in countdowns.iter().enumerate() {
            for &b in &countdowns[i + 1..] {
                let d = a.abs_diff(b);
                let circular = d.min(hop - d);
                assert!(
                    circular >= QUANTUM,
                    "{sr} Hz: two convolvers iterate {circular} samples apart \
                     (countdowns {countdowns:?})"
                );
            }
        }
    }
}
