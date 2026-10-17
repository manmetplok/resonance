# Drums rework: the live checks and outside-the-repo follow-ups

> Stage: **intent**
> Status: **partly built**
> Area: plugins-instruments
> Source: `docs/history/specs/drums-plugin-rework.md` (K3 server task, K4
> re-save, K6b "live 10-minute session before merging", D5 note); memory
> note "Drums rework merged — open: live streaming session, editor live
> look, preset re-save, plok index".

## Why

The drums rework merged on 2026-10-02 with four items that tests cannot
close:

- **Live streaming session.** The spec required a 10-minute live session of
  disk streaming before merging, as for the render pool. No commit or doc
  records one. The stand-ins (saturation and cold-cache throughput tests in
  `plugins/resonance-drums/tests/streaming.rs`) are `#[ignore]`d and need a
  manual release run.
- **Editor live look.** The editor was laid out for the 1571×856 a tiling
  compositor imposes and layout tests cover that size, but no sign-off from
  a real session is recorded.
- **"Drummica Kit" track preset.** The user's track preset predates the
  `params` map (it holds `articulations`, an absolute `kit_path`,
  `overhead_setup_key`, `pad_mic_choices`), so it loads as Stereo and its kit
  reference has no relative path. It must be re-saved once from an instance
  with Drummica selected and Output Mode = Multi.
- **plok.org index.** The copy of `index.json` cached on this machine
  (`~/.local/share/resonance/drumkits/.plok-index.json`) lists five kits.
  Four carry `bytes`, `sha256` and `manifest_sha256`; **Drummica does not**,
  so its download is never checksum-verified and its Update badge falls
  back to the name. IT Techno is not published. The index is generated
  server-side; no generator is in the repo.

## What it would do

Close each item: a recorded live session with no underruns at default
preload, a recorded editor check on the compositor, the re-saved track
preset, and an index where every entry has the optional fields.

## What exists to build on

- Underrun counter and streaming health in the editor's status bar and
  Setup tab (`plugins/resonance-drums/src/stream/mod.rs`).
- `cargo test --release -p resonance-drums --test streaming -- --ignored`.
- `docs/features/plugins-instruments/drums-plugin.md`,
  `docs/features/plugins-instruments/drum-kit-library.md`.

## Open decisions

1. Whether the live session also runs with `RESONANCE_RENDER_THREADS=8` and
   the PipeWire quantum at 64. Recommended: yes, both, since tracking mode
   is the tightest case.
2. Whether to add an index generator script to the repo. Recommended: yes,
   under `scripts/`, so the fields cannot drift per kit.

## Slices

1. Live session: open a Multi Drummica instance, play a dense pattern for
   10 minutes at 128 voices, note underruns and CPU; then the editor
   check at 1571×856 and 960×640.
2. Re-save the track preset (user data, no code).
3. Regenerate and publish `index.json` with full fields; add IT Techno if
   wanted.

## How we'd know it works

Zero underruns in the status bar after the session; the re-saved preset
restores Multi with a relative kit path; the plok.org tab shows the Update
badge by hash for Drummica.
