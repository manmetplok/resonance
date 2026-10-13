# Intelligibility check that ships with the singing skill

> Status: **designed**
> Area: control-api (agent plugin)
> Source: singing skill write-up 2026-10-13; design Q&A with Jorrit
> 2026-10-13 ("script in the plugin skill").

## Why

An agent can't hear a vocal. The only objective check is Whisper word
accuracy, but `scripts/vocal-intelligibility.py` only runs from a repo
checkout and expects an audition `manifest.json`. An agent with the
installed plugin, scoring one bounced vocal, has no usable path.

## What it would do

- `resonance-agent-plugin/skills/singing/scripts/intelligibility.py`:
  - **Input:** `intelligibility.py --text "the lyric as it should be heard" take.wav [more.wav …]`.
    No manifest is needed. `--text-file` takes the lyric from a file;
    `·`, `_`, `AP` and `[..]` blocks are stripped from it so lyric syntax
    can be pasted.
  - **Output:** per file, the word accuracy (1 − WER), what Whisper heard,
    and the words missed. With `--json`, the same as machine-readable
    output.
  - **Running Whisper:** it uses the same Whisper setup as today (`small.en`,
    run through `uv` with the CPU torch index when `whisper` isn't
    importable). If `uv` is missing it says how to install it. On first use
    the model downloads to `~/.cache/whisper`.
  - **Long takes:** files over 30 s are fine (Whisper chunks them), and
    the script trims leading and trailing silence first.
- The singing skill's "Verify without ears" step calls it through
  `${CLAUDE_SKILL_DIR}/scripts/intelligibility.py` on a vocal stem from
  `render_stems {track_ids: [vocal]}` (see
  [render-range-and-stems](render-range-and-stems.md)). It falls back to
  solo + `render_mixdown` when the app lacks `render.stems` in its
  `capabilities`.
- The repo's `scripts/vocal-intelligibility.py` reuses the same scoring
  code (import it, or keep one copy and have the repo script call the
  plugin's), so the audition and the skill score identically.

## Decisions

1. Python, not a Rust method in the app: no new heavy dependency in the
   app; Whisper is an optional, on-demand download.
2. One scoring implementation shared by both scripts.

## Slices

1. The plugin script, the skill text and lockstep, and the repo script
   sharing it. Hermetic test of the WER and normalising code (no Whisper)
   in the script's own self-test (`--self-test`).

## How we'd know it works

`intelligibility.py --text "walk with me along the river" take.wav` on a
known-good audition WAV reports the same accuracy as
`scripts/vocal-intelligibility.py` does for it, and `--self-test` passes.
