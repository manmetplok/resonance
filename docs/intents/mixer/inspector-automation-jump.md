# "Show in Arrange" from the inspector's AUTOMATION rows

> Status: **idea**
> Area: mixer (navigation into timeline)
> Source: `docs/history/specs/mixer-cleanup.md` §3.4 (listed per row; §10 records it as not built); module doc of `resonance-app/src/view/mixer/inspector/automation.rs`

## Why

The mixer inspector lists a channel's automation lanes (target, Read
toggle, remove), but to see or edit a lane's points the user has to switch
to Arrange, find the track and open the right lane by hand.

## What it would do

Each AUTOMATION row gets a jump control that switches to the Arrange view,
scrolls to the owning track (or the master/bus lane area), opens its
automation overlay and selects that lane.

## What exists to build on

- AUTOMATION rows: `resonance-app/src/view/mixer/inspector/automation.rs`
  (emitters only, `AutoChan`, lane resolution from
  `view/mixer/automation.rs`).
- Tab switching and Arrange scroll/viewport messages in the timeline area
  ([`../../features/timeline/`](../../features/timeline/)).

## Open decisions

1. How busses and the master are shown in Arrange. **Recommended:** check
   first whether bus/master lanes have an Arrange home at all; if not,
   offer the jump on track lanes only.
2. One navigation message (`NavigateToLane { owner, target }`) vs chaining
   existing ones. **Recommended:** one message, so it is one testable step.

## Slices

1. Navigation message + handler (tab, scroll, expand overlay, select
   lane) with a timeline-group test.
2. The row's jump glyph, hashed into the inspector fingerprint; golden.

## How we'd know it works

A test dispatches the row's message from the Mix tab and asserts the view
is Arrange, the track is in the viewport and the lane is the selected,
expanded one.
