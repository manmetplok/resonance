//! Pins the `spatial` skill's decay-verification guidance (field report
//! 2026-10-06 §4): read against the **1 kHz band's** T30, with the send
//! raised to 0 dB re the sending track's dry signal, not bare broadband
//! `t30_seconds` on an ordinary-level send. The broadband claim the
//! skills used to make ("within a few percent", "within about 10 %
//! even with the dry signal in") does not hold at ordinary send levels;
//! the 1 kHz band at a raised send does.
//!
//! Params are the `Drum Room` preset's engine voicing
//! (`presets/drum_room.json`), with `decay` overridden to 0.86 s — the
//! knob the field report's investigation measured against — at both the
//! preset's `er_tail_balance` (-0.35) and centred (0). Reference numbers
//! from that investigation, knob 0.86 s, wet 0 dB re dry: broadband
//! 0.68 / 0.71 s, 1 kHz 0.87 / 0.87 s.

use resonance_metering::decay::program_decay;
use resonance_reverb::dsp::Algorithm;

use crate::common::*;

/// The decay the investigation's reference numbers are measured at.
const KNOB: f32 = 0.86;

/// The `Drum Room` preset's engine voicing (`decay` overridden to
/// [`KNOB`]; `er_tail_balance` is set by the caller, since [`Setup::apply`]
/// centres it at 0).
fn drum_room() -> Setup {
    Setup {
        algorithm: Algorithm::Room,
        size: 0.35,
        decay: KNOB,
        damping: 7_000.0,
        diffusion: 0.75,
        er_level: 0.7,
        er_time: 0.45,
        mod_rate: 0.8,
        mod_depth: 0.15,
        low_mult: 1.0,
        low_xover: 250.0,
        high_mult: 0.5,
        build: 0.5,
        width: 1.0,
    }
}

/// Sustained noise through the engine, with the dry signal mixed back in
/// at wet 0 dB re dry (a unity send, the level the skill now tells the
/// agent to raise to before measuring): `stem = dry + wet`, both at unity.
fn sending_track_stem(setup: &Setup, balance: f32) -> (Vec<f32>, Vec<f32>) {
    let mut d = setup.dsp();
    d.set_er_tail_balance(balance);

    const NOISE_S: f32 = 1.5;
    const TAIL_S: f32 = 3.0;
    let n_noise = (NOISE_S * SR) as usize;
    let n_total = n_noise + (TAIL_S * SR) as usize;
    let mut rng = Rng(0xd2a1_c0ff_ee15_c001);
    let (mut l, mut r) = (Vec::with_capacity(n_total), Vec::with_capacity(n_total));
    for i in 0..n_total {
        let x = if i < n_noise { 0.3 * rng.gauss() } else { 0.0 };
        let (wl, wr) = d.process(x, x, setup.diffusion, setup.width);
        l.push(x + wl);
        r.push(x + wr);
    }
    (l, r)
}

/// At a raised send, the 1 kHz band's T30 tracks the knob to within 10 %
/// — the condition the `spatial` skill now prescribes — at both the
/// preset's early-reflection lean and balance centred. Bare broadband
/// `t30_seconds` is printed alongside and is NOT asserted on: it is
/// expected to read short (that is the bug this test guards the fix
/// for).
#[test]
fn room_1khz_t30_tracks_the_knob_at_a_raised_send() {
    println!("Drum Room @ knob {KNOB}s, wet 0 dB re dry: balance  broadband  1kHz");
    for balance in [-0.35f32, 0.0] {
        let setup = drum_room();
        let (l, r) = sending_track_stem(&setup, balance);
        assert_not_silent(&format!("balance {balance}"), &l, &r);

        let d = program_decay(&l, &r, SR);
        assert!(d.found && d.clean, "balance {balance}: {d:?}");

        let mid = d
            .bands
            .iter()
            .find(|b| b.center_hz == 1_000.0)
            .and_then(|b| b.times.t30)
            .unwrap_or_else(|| panic!("balance {balance}: no 1 kHz T30: {d:?}"));
        println!(
            "                                   {balance:>+5.2}    {:>7.3}    {mid:.3}",
            d.times.t30.map_or(f32::NAN, |t| t),
        );

        let err = (mid / KNOB - 1.0).abs();
        assert!(
            err <= 0.10,
            "balance {balance}: 1 kHz T30 {mid:.3} s is {:+.1} % off the {KNOB} s knob",
            100.0 * (mid / KNOB - 1.0)
        );
    }
}
