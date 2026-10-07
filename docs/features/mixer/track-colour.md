# Track colour

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/mixer-cleanup.md` §6 (slice S8).

Every track has a persisted identity colour, shown as a band on its mixer
strip head and its Arrange track header and as the swatch in the
inspector header. A user changes it from the swatch's palette; an agent
uses `track.set_color` with a hex string. Colour is identity only — it
never signals mute, arm, selection or errors.

## Behaviour

- New tracks take the next hue of `theme::TRACK_PALETTE` (ten hues:
  coral, amber, sand, olive, jade, teal, cornflower, periwinkle, orchid,
  rose) by their order (`track_palette_color`).
- A project saved before colours existed opens coloured: a track without a
  colour gets one deterministically from its order.
- Sub-tracks always draw and report their parent's colour, looked up at
  draw time (`TrackRegistry::display_color`); a load normalises stored
  sub-track colours to the parent's.
- Busses and the master have no per-channel colour: busses get a warm head
  band, the master a neutral one, and busses report no colour on the wire.
- In Arrange, a track group's colour still draws as the cluster rail and
  the track's band sits inside it.
- The inspector palette is an inline card under the header (closes on Esc
  or a press elsewhere). Any `#rrggbb` is accepted over the control API,
  not only palette hues.
- `SetTrackColor` is one undo step and is saved as
  `ProjectTrack::color: Option<[u8; 3]>`.
- The colour is hashed into the strip and inspector fingerprints.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Palette | `resonance-app/src/theme.rs` | `TRACK_PALETTE`, `track_palette_color`, `track_color`, `TRACK_COLOR_BAND_WIDTH` |
| State | `resonance-app/src/state/tracks.rs` | `TrackState::color`, `TrackRegistry::display_color` |
| Update | `resonance-app/src/update/track.rs`, `update/chain_ui.rs` | `TrackMessage::SetTrackColor`, palette open/close |
| Persistence | `resonance-app/src/project/model.rs` | `ProjectTrack::color` |
| Control | `resonance-control/src/methods/track.rs`, `resonance-app/src/update/control/view_model/track.rs` | `SetColorParams`, `parse_hex_color`, `format_hex_color` |
| View | `resonance-app/src/view/mixer/inspector/mod.rs`, `view/mixer/strip_parts.rs`, `view/track_header/track.rs` | `color_swatch`, `color_palette` |

## Surfaces

- **GUI:** inspector header swatch → palette; band on strip heads and
  Arrange track headers.
- **Control API / MCP:** `track.set_color` → `mcp__resonance__track_set_color`
  (`"#rrggbb"`, `#` optional); `song.tracks` reports `color` (hex) for
  tracks and `null` for busses.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Persistence, legacy load, sub-track normalisation | `resonance-app/tests/io/track_color_persist.rs` | `./scripts/run-tests.py -p resonance-app` |
| Strip band, inspector swatch | `resonance-app/tests/mixer/{mixer_strip_anatomy,inspector_chain_row}.rs` | same |
| Control | `resonance-app/tests/control/control_track_mixer.rs` | same |

Not covered: no golden shows the open colour palette.

## Decisions and deviations

- Only tracks carry a colour (spec allowed busses to follow later; they
  did not).
- Sub-tracks follow the parent rather than keeping their own colour.

## Known gaps

- None recorded.
