# Reference loudness match that actually matches

> Status: **partly built**
> Area: mixer
> Source: design `docs/intents/prototypes/reference-track-ab-monitoring/DESIGN.md` ("Loudness match"); found while documenting the mixer (2026-10-07)

## Why

The reference panel has a Loudness-match toggle whose whole purpose is to
stop a louder master from sounding "better" in an A/B. Everything around it
is wired — the toggle, undo, persistence, the engine command, the
`RefLoudnessMatchChanged` echo and the offset readout — but the offset is
never computed. `ReferenceEntry::offset_db` in
`resonance-audio/src/engine/reference.rs` is initialised to 0 and nothing
ever writes it; the analysis worker's "ComputingOffset" stage only reports
progress ("a later todo"). So matching is a silent no-op and the readout
always says 0 dB.

## What it would do

With matching on, the active reference plays at a gain that puts its
integrated loudness level with the mix's, shown as e.g.
"−9.8 → −14.0 LUFS = −4.2 dB", with the manual trim on top.

## What exists to build on

- Reference integrated LUFS, measured at load
  (`run_reference_analysis` → `ReferenceEntry::integrated_lufs`).
- The mix's live loudness from the A/B mix meter
  (`callback/master_pass.rs` → `shared.mix_meter`, `MeterSnapshot`).
- The gain path that already applies `offset_db + trim`
  (`ReferencePlayer`, `engine/reference.rs` ~line 199).
- `meter.measure` can measure the master offline for a stable number.
- Feature doc: [`../../features/mixer/reference-ab-monitoring.md`](../../features/mixer/reference-ab-monitoring.md).

## Open decisions

1. Which mix loudness to match against. Options: the live mix meter's
   integrated value (moves while playing, empty before playback), or an
   offline measurement of the whole mix. **Recommended:** live integrated
   loudness since playback started, recomputed when matching is toggled on
   and on transport start; fall back to the last offline measurement when
   no live value exists; show which one is used.
2. Re-match continuously or on demand. **Recommended:** on demand (toggle /
   "re-match" click), so the reference level does not drift while the user
   listens.

## Slices

1. Engine: compute and store `offset_db` for the active reference when
   matching is enabled (and when the active reference changes); emit it in
   `RefLoudnessMatchChanged`. Engine test with a known mix loudness.
2. Panel: show source loudness → target loudness = offset; golden.

## How we'd know it works

- `resonance-audio/tests/engine/reference_handlers.rs` gains a case: a
  reference at −9 LUFS against a −14 LUFS mix reports about −5 dB and the
  monitored reference meters at about −14 LUFS.
- `reference_panel_loudness_readout` golden shows a non-zero offset.
