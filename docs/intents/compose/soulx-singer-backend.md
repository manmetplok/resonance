# SoulX-Singer as a second singing backend

> Status: **idea**
> Area: compose (vocal rendering)
> Source: research report
> [singing-voicebank-options-2026](../../history/research/singing-voicebank-options-2026.md) (2026-10-07); split out of
> [vocal MCP control and quality](vocal-mcp-control-and-quality.md) D31.

## Why

Every voice Resonance can sing with is a DiffSinger bank. English
DiffSinger banks are scarce, mostly restrictively licensed, and expose few
expression controls (none of the shipped three takes energy or
breathiness). SoulX-Singer (Soul AI Lab, released 2026-02-06, arXiv
2602.07803) is the only open 2026 model found that:
- has English;
- takes note-level score input;
- ships both code and weights under a permissive licence (Apache-2.0).

It is a flow-matching model trained on about 42k hours, about 20k of them
English. It clones a voice from a reference clip without training. Its
melody mode can also follow a pitch curve we supply.

## What it would do

A time-boxed experiment that answers whether SoulX-Singer is worth a
production backend:
- **Hosting:** run it as a local Python sidecar process behind the same
  render-unit interface `vocal_svs` uses for DiffSinger.
- **Comparison:** render the slice-6 audition song from the vocal spec and
  compare it blind against the best DiffSinger banks.
- **Measurements:** CPU and GPU render time, memory, and its English
  phoneme set compared with our CMU G2P output.

## What exists to build on

- The render-unit plan and cache in `resonance-app/src/compose/vocal_svs/`,
  and the bank registry the vocal spec introduces (slice 4).
- The audition script from the vocal spec (slice 6).

## Open decisions

1. **Sidecar or ONNX:** a sidecar (recommended for the experiment) or our
   own ONNX export. No ONNX export is published.
2. **Pitch:** use score mode, where SoulX-Singer predicts the pitch, or
   melody mode, where Resonance supplies the pitch curve. The recommendation
   is melody mode, which keeps our vibrato and portamento.
3. **Licence:** confirm Apache-2.0 from the LICENSE files themselves. The
   research read it from READMEs and model cards.

## Slices

1. Sidecar spike: render one phrase from a score, and measure time and
   memory.
2. Backend adapter behind the bank registry, and audition.

## How we'd know it works

The blind audition ranks it, and a 4-bar phrase renders on CPU in a time we
accept for interactive work (target to be set from the measurements).
