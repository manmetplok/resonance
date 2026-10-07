# Arrange view: lanes, track headers, scrolling and zoom

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none as one spec; layout rules came from epic #36 (doc #203),
> review findings VIEW-10/21/33 and FU-D1/FU-V3a/FU-V3b in
> `docs/history/reviews/`.

The Arrange tab is the horizontal timeline: a column of track headers on the
left and a canvas of lanes on the right, with the ruler, section band and
global-tracks shelf pinned above them. This doc covers the frame those
features sit in — which rows exist, how they line up, how the view scrolls,
zooms and follows the playhead, and how it stays fast with many tracks. What
happens *inside* a lane (clips, automation, takes, markers…) has its own doc;
see the [README](README.md).

## Behaviour

### Rows

- Top to bottom: ruler (28 px), section band (22 px, only when the song has
  section placements), global-shelf header (32 px, always), the chord / tempo
  / signature lanes (56 + 40 + 28 px, only when the shelf is expanded), then
  the track rows. The total of these is `Resonance::arrange_header_offset`.
- Track rows come from one shared model, `ArrangeRowLayout`, which emits in
  display order:
  - `GroupHeader` — 60 px folder row ([track-groups.md](track-groups.md));
  - `Track` — 96 px lane per top-level track;
  - `AutomationLane` — 44 px sub-row per automation lane while the track's
    automation is expanded ([automation-lanes.md](automation-lanes.md));
  - `TakeRow` — 38 px sub-row per take while the track's take lane is open
    ([takes-and-comping.md](takes-and-comping.md)).
  Members of a collapsed group are left out entirely.
- Instrument sub-tracks (fan-out ports, e.g. drum outs or vocal stacks) are
  never shown in Arrange; they appear only in the Mixer.
- Track order is the persisted `TrackState::order`. There is no way to
  reorder tracks from the UI.

### Track header

Each 96 px header (280 px wide column) shows, left to right: group rails (if a
member), the 4 px track-colour band, a selection / record-arm stripe, a 28 px
instrument glyph, name and kind, a delete button in the corner, and a row of
mini buttons — **freeze, mute, solo, arm, monitor** — plus carets for the
automation sub-rows and the take lane. On a frozen track arm and monitor are
locked. Each button's tooltip names the matching selection command and chord.
Right-click opens the track menu: freeze / unfreeze, freeze selected, freeze
all, *Save track as preset…*, reveal freeze cache. Renaming and recolouring a
track happen in the Mixer, not here. Deleting a track with clips asks for
confirmation first (`view/confirm_delete_track.rs`).

### Scrolling

- **Horizontal:** the canvas is as wide as the song (last clip end plus 8
  bars, at least 800 px) inside an iced `Scrollable` (`ARRANGE_SCROLL_ID`).
  The `Scrollable` owns the horizontal offset; `ArrangeViewport::scroll_offset`
  is only its read-back. A horizontal wheel or trackpad delta goes to it.
- **Vertical:** handled inside the canvas (so the lanes move under the pinned
  ruler and shelf). A wheel line is 30 px. The offset is clamped to the layout
  height, including automation, take and group rows (VIEW-21). The canvas
  draws its own vertical scrollbar at the right edge of the *visible* window.
- Loading a project scrolls back to the start with a real `scroll_to(0)`; an
  undo or redo leaves the scroll where it was.

### Zoom

- Horizontal zoom is `ArrangeViewport::zoom` in pixels per second: default
  100, each step ×1.5 or ÷1.5, clamped to 10–1000.
- Zoom in / out: the floating buttons at the bottom-right of the timeline,
  Cmd/Ctrl+= and Cmd/Ctrl+-. The mouse wheel does **not** zoom.
- There is no vertical (track-height) zoom. Zoom and scroll are not saved with
  the project.
- The snap grid follows zoom (beats, bars or multi-bar steps); see
  [clip-editing.md](clip-editing.md).

### Playhead follow

While playing on the Arrange tab, when the playhead leaves the visible width
the view pages so the playhead sits 5 % in from the left edge. Follow is:
off when the *Follow playhead* setting is off (default on); paused by a manual
horizontal scroll during playback until the playhead comes back into view or
the transport stops; skipped while any pointer drag on the timeline is in
progress (clip move/trim, fade, gain, loop marker, browser drop).

### Performance

- The canvas keeps a geometry cache keyed by a content fingerprint; the
  playhead and the recording overlay are drawn in an uncached overlay pass so they never
  invalidate it.
- The cached pass only draws clips inside a quantized window around the
  visible area (`cull.rs`); the window is part of the fingerprint and is taken
  from `ViewportProbe` in the same frame, so nothing blank can scroll into
  view.
- The header column only builds widgets for rows inside the visible height,
  so a 200-track project allocates a handful of header cells.
- The row layout is memoized per frame (`content_fingerprint_memo` and the
  layout memo).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Viewport state | `resonance-app/src/state/viewport.rs` | `ArrangeViewport`, `ARRANGE_SCROLL_ID`, `ViewMode` |
| Scroll / zoom reducers | `resonance-app/src/update/viewport.rs` | `ViewportMessage`, `zoom_in`, `zoom_out`, `scroll_to_y`, `max_scroll_y`, `arrange_scrolled` |
| Playhead follow | `resonance-app/src/update/tick.rs` | `follow_playhead`, `arrange_pointer_drag_active`, `FOLLOW_LEAD_FRACTION` |
| Row layout | `resonance-app/src/state/arrange_layout.rs`, `state/arrange.rs` | `ArrangeRowLayout`, `ArrangeRowKind`, `arrange_row_layout`, `arrange_header_offset`, `track_id_at_arrange_y` |
| Canvas | `resonance-app/src/view/timeline/mod.rs`, `view/timeline_panel.rs` | `TimelineCanvas`, `TimelineState`, `TimelineFingerprint`, `content_width_natural`, `view_timeline` |
| Canvas input | `resonance-app/src/view/timeline/input/` | `pointer.rs` (press routing), `hover.rs` (`handle_wheel`, cursors), `keyboard.rs` |
| Culling / probe | `resonance-app/src/view/timeline/cull.rs`, `viewport_probe.rs`, `scrollbar.rs` | `ViewportProbe` |
| Header column | `resonance-app/src/view/track_header/mod.rs`, `track.rs`, `shelf.rs` | header virtualization, `view_track_header`, global-shelf labels |
| Track context menu | `resonance-app/src/view/menus.rs` | `track_menu_item` |
| Constants | `resonance-app/src/theme.rs` | `TRACK_HEIGHT`, `GROUP_HEADER_HEIGHT`, `AUTOMATION_LANE_ROW_HEIGHT`, `TAKE_ROW_HEIGHT`, `RULER_HEIGHT`, `TRACK_HEADER_WIDTH` |

Invariants: the header column, the canvas draw, the canvas hit-testing and
the scroll clamp must all read the same `ArrangeRowLayout` — never
`index * TRACK_HEIGHT`. Pointer → sample conversions must not add
`scroll_offset` (the canvas works in content coordinates, VIEW-10). The header
column paints opaque chrome on top of the lanes so negatively-padded rows
never bleed into the ruler (iced `clip(true)` does not clip backgrounds).

## Surfaces

- **GUI:** Arrange tab (Cmd/Ctrl+1). Zoom buttons, Cmd/Ctrl+= / -, global
  shelf toggle (click its header or Cmd/Ctrl+Alt+G), browser toggle
  (Cmd/Ctrl+Alt+B), *Follow playhead* in Settings.
- **Control API / MCP:** none for the view itself; the agent reads the song
  through `song_summary` / `song_tracks` instead (see
  [method-catalogue.md](../control-api/method-catalogue.md)). Track add / delete / rename / colour
  tools (`track_add`, `track_delete`, `track_rename`, `track_set_color`)
  belong to the mixer/track docs in [channel-strips-and-inspector.md](../mixer/channel-strips-and-inspector.md) and [track-colour.md](../mixer/track-colour.md).
- **Agent skills:** none specific.

## How it's tested

| What | Where | Run |
|---|---|---|
| Row layout model | `resonance-app/tests/timeline/arrange_layout.rs`, `layout_memo.rs` | `./scripts/run-tests.py -p resonance-app` |
| Scroll clamp, scrollbar, load reset | `tests/timeline/vertical_scroll_clamp.rs`, `vertical_scrollbar_visible.rs`, `load_scroll_reset.rs` | same |
| Playhead follow | `tests/timeline/playhead_follow.rs` | same |
| Culling and cache fingerprint | `tests/timeline/arrange_cull.rs`, `render_cache.rs` | same |
| Header alignment / virtualization goldens | `tests/timeline/track_header_alignment.rs` → `tests/snapshots/track_header_alignment_scroll_*`, `track_header_no_bleed_into_chrome_expanded-wgpu.png`, `track_header_virtualizes_100_tracks_scroll_0-wgpu.png`, `timeline_long_song_vertical_scrollbar-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test timeline` |

Not covered: zoom has no dedicated test (only the clamp arithmetic in
`update/viewport.rs`); horizontal wheel routing to the `Scrollable` is not
exercised (iced handles it); there is no test of the track context menu
contents.

## Decisions and deviations

- **Horizontal scroll belongs to an outer `Scrollable`, vertical to the
  canvas.** The ruler and shelf must stay pinned vertically but scroll
  horizontally with the lanes; splitting the axes gives that without a custom
  scroll container.
- **Page-style follow**, not continuous scrolling, so the content under the
  cursor stays still between page turns.
- **The Arrange header is for arranging.** Mono, FX bypass and
  bounce-in-place stay on the mixer strip; freeze is the one channel control
  shown in both places.

## Known gaps

- No track reordering — see
  [../../intents/timeline/track-reordering.md](../../intents/timeline/track-reordering.md).
- No wheel zoom, no zoom-to-fit, no vertical zoom; zoom is not saved with the
  project. No intent filed (no spec asks for them).
