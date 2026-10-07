# Frozen tracks must not apply their aux sends twice

> Status: **idea** (bug read from the code; not yet reproduced by a test)
> Area: audio-engine (see `docs/features/audio-engine/track-freeze.md`)
> Source: `docs/history/reviews/field-report-2026-10-06.md` header
> ("Open follow-up from §3"); confirmed by reading master @ `8eac71b3`.
> Register items covered: field report 2026-10-06 §3.

## Why

A frozen track should sound exactly like the unfrozen one. Today a frozen
track that sends to a reverb or delay return very likely gets that effect
twice: once baked into its freeze cache, and again live from the cache. The
reverb is louder after freezing, and the baked part ignores the send level's
post-fader scaling. Kit taps (sub-tracks) now send like any track
(`8cc428b5`), so a frozen drum kit with taps on a room bus is affected too.

Evidence in the code:

- `engine/bounce/freeze.rs::to_freeze_cache` builds its `ChunkCtx` with
  `send_filter: None`.
- `mixer/render/track_pass.rs::replay_route` applies
  `apply_track_aux_sends` whenever the filter is not `SendFilter::Dry`, so
  the frozen track's sends are tapped during the freeze render. Under
  `freeze_raw` the track gains are `(1.0, 1.0)`, so post-fader sends tap at
  unity instead of at the fader.
- `RenderStrategy::bus_disposition` renders every unmuted bus in a bounce,
  and the bus pass sums busses into master, so the return's wet output lands
  in the cache.
- On playback, `render_track_source` substitutes the cache, and the same
  reduction applies the track's sends again from the cache signal (which
  already contains the wet return).

## What it would do

A freeze cache holds only the track's own post-FX signal (and its
sub-tracks'), with no send or return content. Frozen playback, export and
stems are sample-identical to the unfrozen track, sends included.

## What exists to build on

- `SendFilter::Dry` already exists ("the tracks render as usual but tap no
  aux send at all") and is used by the depth measurement.
- `RenderStrategy::send_filter` / `ChunkCtx::send_filter` take a
  `(names, SendFilter)` pair.
- Tests: `tests/bounce/freeze_render_core.rs`,
  `tests/mixer/freeze_playback_substitution.rs`,
  `tests/mixer/sub_track_aux_sends.rs` (sub-track send harness).

## Open decisions

1. Fix in the freeze render (pass `SendFilter::Dry` for the frozen track
   and its sub-tracks) or skip the bus pass entirely under `freeze_raw`?
   **Recommended:** `SendFilter::Dry`; it is the existing, tested mechanism
   and keeps other bus behaviour unchanged.
2. Existing caches on disk contain the baked return. **Recommended:** bump
   what the fingerprint covers (or a cache format version) so existing
   frozen tracks show as stale and offer a refreeze.

## Slices

1. Failing test first: a track with a post-fader send to a return carrying a
   simple gain plugin; freeze, play frozen, compare to unfrozen bit for bit
   (and the same with a kit tap sending).
2. Pass `SendFilter::Dry` in `to_freeze_cache`; the test passes; run the
   suite at `RESONANCE_RENDER_THREADS=8`.
3. Invalidate old caches (fingerprint/version) and check the app's stale
   banner appears for them.

## How we'd know it works

The new freeze test is bit-identical frozen vs unfrozen with sends; the
existing freeze goldens are unchanged for tracks without sends.
