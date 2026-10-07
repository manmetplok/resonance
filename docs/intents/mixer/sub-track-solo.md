# Sub-track solo that does something

> Status: **idea**
> Area: mixer
> Source: found while documenting the mixer (2026-10-07): `view/mixer/track_strip.rs` draws a solo button on sub-track strips; `mixer/render/strategy.rs::sub_track_silenced` ignores it

## Why

An expanded multi-output instrument (a drum kit) shows one strip per
output port, each with M and S. Mute works. Solo lights up, is saved and
is undoable, but changes nothing audible: the engine excludes sub-tracks
from the global solo scan (`any_top_level_solo`) and a sub-track follows
its parent's solo, never its own. "Solo the snare" is a basic mixing move
and the button suggests it works.

## What it would do

Pressing S on a sub-track makes only that tap (plus anything else soloed)
audible: the rest of the kit and every other unsoloed track drop out. Done
when the live mix and `render.mixdown` agree on it.

## What exists to build on

- Solo resolution: `resonance-audio/src/mixer/render/strategy.rs`
  (`track_silenced`, `sub_track_silenced`, `sub_track_disposition`) and
  `types/track.rs` (`any_top_level_solo`, `snapshot_top_level_solo`).
- The engine test that pins today's behaviour on purpose:
  `resonance-audio/tests/mixer/solo_predicate.rs::sub_track_solo_alone_is_false`
  (a soloed sub-track must not silence unrelated top-level tracks); any
  fix must keep that test's intent while making the solo audible.
- Feature doc: [`../../features/mixer/volume-pan-mute-solo.md`](../../features/mixer/volume-pan-mute-solo.md).

## Open decisions

1. Fix or remove. **Recommended:** fix — the alternative (hide S on
   sub-strips) removes a useful move.
2. Semantics: a soloed sub-track counts as "something is soloed"; its
   parent must still *render* (it produces the tap's audio) but its own
   port 0 and its unsoloed siblings are silenced. **Recommended:** model it
   like the existing `discard_own_output` fan-out-driver disposition used
   by stems.
3. Whether `mixer.set_solo` on a sub-track id should be accepted.
   **Recommended:** yes, same rule.

## Slices

1. Engine: solo aggregate includes sub-tracks; parent becomes a fan-out
   driver when only children are soloed; live and bounce share it.
   Update `solo_predicate.rs` deliberately.
2. App: no change needed beyond tests (`ToggleSolo` already sends the
   flag); a mixer-group test and a control test.

## How we'd know it works

- Render test: kit with kick and snare taps plus another track; solo the
  snare tap → only snare audio in live and offline renders.
- Existing solo race / predicate tests still pass after the deliberate
  update.
