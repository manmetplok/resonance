# Scroll the arrangement to a track

> Status: **idea**
> Area: ui-shell (command palette) / timeline
> Source: `docs/history/specs/command-palette.md` §7.4 as-built note ("`#` selects the track but does not scroll the arrangement to it"); still true at `8eac71b3`.

## Why

In a long project, typing `#bass` in the palette selects the bass track. If
that track is below the visible rows, the user sees nothing change and has
to scroll by hand to find it. The palette's `#` mode is meant for
navigation, so it should take the user there.

## What it would do

- Running a `#` row selects the track **and** scrolls the Arrange view
  vertically just enough to show its row.
- If the user is in another view, running the row switches to Arrange
  first.
- A track inside a collapsed group either expands the group, or scrolls to
  the group row (see decision 2).
- The same primitive is available for other callers, such as a future "reveal
  selected track" command.

## What exists to build on

- `resonance-app/src/update/palette.rs::run`: `PaletteItem::Track(id)` emits
  only `UiMessage::SelectTrack(Some(id))`.
- `resonance-app/src/state/arrange_layout.rs`: `ArrangeLayout::track_row_rect(track_id)`
  returns a row's y extent.
- `resonance-app/src/update/viewport.rs::scroll_to_y`: clamps and sets
  `viewport.scroll_offset_y`. The arrange canvases scroll vertically through
  this value, not through an iced `Scrollable`.
- `docs/features/ui-shell/command-palette.md`.

## Open decisions

1. Should the target row sit at the top, the centre, or the nearest edge?
   **Recommend nearest edge**: don't move when the row is already visible.
2. What about a track in a collapsed group? **Recommend scrolling to the
   group row** and leaving collapse state alone, since collapsing is a user
   choice.
3. Should Arrange-only scrolling also apply in the Mixer, scrolling the strip
   lane? **Recommend yes, as a later slice.**

## Slices

1. Add a `UiMessage::RevealTrack(id)` reducer. It looks up `track_row_rect`
   and calls `scroll_to_y`, and the palette `#` row emits it after
   `SelectTrack`. Test: a 100-track project, run `#Track 90`, assert that
   `scroll_offset_y` puts the row in view.
2. Add a "Reveal Selected Track" command in the registry, with no default
   chord.
3. Mixer strip-lane reveal.

## How we'd know it works

- A reducer test in `resonance-app/tests/performance/command_palette.rs`.
- The `track_header_virtualizes_100_tracks_*` style golden after a reveal.
