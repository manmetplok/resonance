# Opening a project at a different sample rate

> Stage: **intent**
> Status: **idea**
> Area: hardware-io
> Touches: project-core
> Source: code reading: `ProjectFile::sample_rate` is written in `update/project_io/serialize.rs` and never read; clip, loop and marker positions are stored in samples.

## Why

The project file stores audio clip starts (`ProjectClip::start_sample`),
MIDI clip starts, the loop range and other positions in samples. If the
engine runs at a different rate than the one the project was saved at (a
device that cannot do 48 kHz, `RESONANCE_NO_GRAPH_FORCE`, another machine),
the clip audio is resampled at load (`ClipSource::open_wav_at_rate`) but
every position keeps its sample number. Everything lands at the wrong time
(9 % early or late between 44.1 and 48 kHz), and the next save writes the
wrong positions back. The engine pins 48 kHz when it can, so this is rare,
but silent when it happens.

## What it would do

On open, compare `ProjectFile::sample_rate` with the engine rate and
rescale every sample-denominated field by the ratio (or warn and refuse),
so a project opens musically identical at any rate.

## What exists to build on

- `ProjectFile::sample_rate` is already saved.
- The tempo map converts bars ↔ samples at any rate.
- WAV content already resamples on load
  (`resonance-audio/tests/io/load_wav_rate_mismatch.rs`).

## Open decisions

1. Rescale silently, rescale with a banner, or refuse? Recommended: rescale
   and show a one-line banner naming both rates.
2. Store positions musically instead (bars/ticks)? Recommended: no, too
   large; rescaling at load is enough.

## Slices

1. Rescale pass in project load (clips, MIDI clips, loop, markers, automation
   breakpoints, takes) with a test that round-trips 44.1 → 48 kHz.
2. Banner and a note in the project-format doc
   ([../../features/project-core/](../../features/project-core/)).

## How we'd know it works

A test in `resonance-app/tests/io/` that saves at one rate and opens at
another and checks every position against its bar; a manual open with
`RESONANCE_NO_GRAPH_FORCE=1` on a 44.1 kHz graph.
