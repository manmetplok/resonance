#!/usr/bin/env python3
"""FU-D1b1: rescale the factory presets' envelope times for DSP2-12.

Commit 3c670176 ("Wavetable: envelope times are times to target") changed
what `amp_attack`/`amp_decay`/`amp_release` (and the parallel `mod_*`
envelope) mean, without changing the presets: the param used to be a
one-pole time constant with an overshoot target, and is now the time to
reach the target exactly. That made every factory preset's decay/release
~6x shorter and its attack ~1.5x shorter (see the commit message). This
script is the one-time migration that puts the *durations* back the way
they sounded before, now expressed in the new, correct units. It was run
once against `presets/*.json` on this repo and does not need to be run
again; it is kept here so the conversion is reproducible and auditable
rather than a one-off hand edit.

The math (restated in `tests/factory_preset_envelope_times.rs`, which
checks the result):

Old model (pre-3c670176), curve 0 (every factory preset's `amp_curve` and
`mod_curve` are 0, asserted below): shape = (1+curve*0.8).max(0.2) == 1,
so tau (seconds) == the param itself. Each stage is a one-pole glide from
a start level toward a fixed overshoot target:

  attack:  target 1.3,            from 0.0  -> peak (level 1.0)   at tau*ln(1.3/0.3)
  decay:   target sustain-0.001,  from 1.0  -> sustain (+1e-4)    at tau*ln((1-sustain+0.001)/0.0011)
  release: target -0.001,         from 1.0  -> -60 dB (level 1e-3) at tau*ln(1.001/0.002)

New model (3c670176): `EnvCoeffs::for_params`/`coeff_for` solves the
coefficient so the stage lands on its label exactly, by construction —
independent of sustain or curve for attack and release, and exactly
`decay_s` for decay regardless of sustain (see `src/dsp/envelope.rs`).
So preserving the physical duration is just:

  new_attack  = old_attack  * ln(1.3 / 0.3)                    (~1.4663x)
  new_decay   = old_decay   * ln((1 - sustain + 0.001)/0.0011)  (sustain-dependent)
  new_release = old_release * ln(1.001 / 0.002)                 (~6.2156x)

Each result is clamped to the param's declared range (attack 0.001-5.0 s,
decay/release 0.001-10.0 s — `src/params/env.rs`); a handful of slow pads
and the drone preset hit the ceiling and lose a little of their former
length rather than exceed it (printed below as they're found).

Usage: python3 rescale_envelope_times_dsp2_12.py <presets-dir>
"""
import glob
import math
import os
import re
import sys

ATTACK_FACTOR = math.log(1.3 / 0.3)  # ~1.466337
RELEASE_FACTOR = math.log(1.001 / 0.002)  # ~6.215608


def decay_factor(sustain: float) -> float:
    span = 1.0 - sustain
    return math.log((span + 0.001) / 0.0011)


ATTACK_RANGE = (0.001, 5.0)
DECAY_RANGE = (0.001, 10.0)
RELEASE_RANGE = (0.001, 10.0)


def clamp(v: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, v))


def round_sig(v: float, sig: int = 4) -> float:
    if v == 0:
        return 0.0
    d = math.floor(math.log10(abs(v)))
    factor = 10 ** (sig - 1 - d)
    return round(v * factor) / factor


def fmt(v: float) -> str:
    # Match the terse style already in the presets: no needless trailing zeros.
    s = f"{v:.6f}".rstrip("0").rstrip(".")
    if s in ("", "-0"):
        s = "0.0"
    if "." not in s:
        s += ".0"
    return s


def main() -> None:
    preset_dir = sys.argv[1]
    clamp_notes = []

    for path in sorted(glob.glob(os.path.join(preset_dir, "*.json"))):
        text = open(path).read()
        name = os.path.basename(path)
        changed = {}

        for prefix in ("amp", "mod"):
            a_k, d_k, s_k, r_k, c_k = (
                f"{prefix}_attack",
                f"{prefix}_decay",
                f"{prefix}_sustain",
                f"{prefix}_release",
                f"{prefix}_curve",
            )
            curve_m = re.search(r'"' + c_k + r'":\s*([0-9.eE+-]+)', text)
            assert curve_m, f"{name}: no {c_k}"
            curve = float(curve_m.group(1))
            assert curve == 0.0, f"{name}: {c_k} != 0 ({curve}); conversion assumes curve 0"

            def get(key):
                m = re.search(r'"' + key + r'":\s*([0-9.eE+-]+)', text)
                assert m, f"{name}: no {key}"
                return float(m.group(1))

            old_a, old_d, old_s, old_r = get(a_k), get(d_k), get(s_k), get(r_k)
            raw_a = old_a * ATTACK_FACTOR
            raw_d = old_d * decay_factor(old_s)
            raw_r = old_r * RELEASE_FACTOR
            new_a = round_sig(clamp(raw_a, *ATTACK_RANGE), 4)
            new_d = round_sig(clamp(raw_d, *DECAY_RANGE), 4)
            new_r = round_sig(clamp(raw_r, *RELEASE_RANGE), 4)

            for label, raw, rng in ((a_k, raw_a, ATTACK_RANGE), (d_k, raw_d, DECAY_RANGE), (r_k, raw_r, RELEASE_RANGE)):
                if raw < rng[0] or raw > rng[1]:
                    clamp_notes.append(f"{name}: {label} raw {raw:.4f} clamped to {rng}")

            changed[a_k] = (old_a, new_a)
            changed[d_k] = (old_d, new_d)
            changed[r_k] = (old_r, new_r)

        new_text = text
        for key, (_old_v, new_v) in changed.items():
            pattern = re.compile(r'("' + re.escape(key) + r'":\s*)([0-9.eE+-]+)')
            new_text, n = pattern.subn(lambda m, nv=new_v: m.group(1) + fmt(nv), new_text, count=1)
            assert n == 1, f"{name}: failed to substitute {key}"

        if new_text != text:
            open(path, "w").write(new_text)
        print(f"{name}: " + ", ".join(f"{k}={v[0]}->{v[1]:.4f}" for k, v in changed.items()))

    print()
    print(f"ATTACK_FACTOR={ATTACK_FACTOR:.6f}  RELEASE_FACTOR={RELEASE_FACTOR:.6f}")
    if clamp_notes:
        print("Clamped (hit the param range ceiling, duration shortened vs. the old sound):")
        for n in clamp_notes:
            print(f"  {n}")


if __name__ == "__main__":
    main()
