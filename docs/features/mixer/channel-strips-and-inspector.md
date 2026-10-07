# Channel strips and the mixer inspector

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/mixer-cleanup.md` (§10 lists the as-built deviations).

The Mix tab shows one strip per track, bus and the master, plus an
inspector on the right that edits whichever channel is selected. The rule
is "the strip shows state, the inspector edits": a strip answers what is on
a channel and whether it is active, and keeps only the controls you ride
live (mute, solo, arm, monitor, the chain on/off switch, pan, fader).
Every structural edit — add, remove, reorder, replace, presets, open a
plugin, automation lanes, routing, sends, track options — lives in the
inspector.

## Behaviour

### Layout

- Top lane: track strips (horizontally scrollable), then the master strip
  pinned on the right. Bottom lane: bus strips, then a small `+ Bus` strip.
  The inspector (320 px) sits right of both lanes. The Reference & A/B rail
  (360 px) opens to the right of the inspector when the chrome `REF`
  toggle is on (see [reference-ab-monitoring.md](reference-ab-monitoring.md)).
- Track order follows `TrackState.order`. A track group folds into one
  coloured cluster at the slot of its first member (group strips and
  macro mute/solo/level belong to track groups, see
  [track-groups.md](../timeline/track-groups.md)).
- A parent with sub-tracks (multi-output instrument) can be expanded; its
  sub-track strips follow it flush, ordered by `output_port_index`.
  Collapsed, the parent shows a column of mini meters for its sub-tracks.

### Strip anatomy

- Track strip, 160 px wide (`MIXER_STRIP_WIDTH`): head with a 4 px
  track-colour band, glyph and single-line name; one button row (M / S /
  arm / monitor); the `FX` header, which is the whole-chain bypass switch;
  one line per chain slot; a centred pan knob with its value under it;
  the fader with a stereo peak meter.
- Slot lines never wrap: 11 px text, ellipsised at 17 characters
  (`MIXER_SLOT_LINE_CHARS`). Dot states: `●` active, `○` bypassed (dimmed),
  pink `●` for a missing or unavailable plugin. When the whole chain is
  bypassed every line dims. The instrument slot is drawn in the accent
  colour with a hairline under it; an instrument track with no instrument
  shows a dim `No instrument` line.
- Click a slot line: select the owner and focus that slot in the
  inspector's CHAIN. Double-click: open the plugin window (see
  [plugin-chains.md](plugin-chains.md)). Click on `No instrument`: cue the
  CHAIN `+ Add instrument` picker (accent border plus a "Pick an
  instrument" line; iced cannot focus a `pick_list`).
- Sub-track strips are 104 px, same anatomy, slot names at 10 characters.
  They draw their parent's colour.
- Bus strips use the same anatomy at 160 px, with a warm head band, mute in
  the head row and **no solo** (busses have no solo state). Delete lives in
  the inspector.
- The master strip has a neutral head, the FX switch, slot lines and the
  fader. No pan, mute or solo.
- External-instrument tracks keep their `Ext` pill, offline flag and three
  summary chips on the strip (see [external-instruments.md](../hardware-io/external-instruments.md)).
- Double-clicking a track or bus name starts an inline rename. Enter or a
  click elsewhere commits, Esc cancels. One rename is open at a time and
  only the surface that was double-clicked (strip head or inspector header)
  draws the field. A commit is its own undo step; an untouched field
  follows a rename made elsewhere instead of writing the old name back.

### Selection

Selecting a track, a bus or the master puts it in the inspector. The three
selections are mutually exclusive (`selected_track` on the interaction
state, `MixerUiState::selected_bus`, `MixerUiState::selected_master`). A
project load clears the master selection, the focused slot and every
transient CHAIN affordance (`MixerUiState::reset_for_project`).

### Inspector

- Track: header (name with inline rename, colour swatch, type tag), then
  CHAIN → SENDS → ROUTING → AUTOMATION → TRACK.
- Bus: CHAIN → ROUTING → AUTOMATION → BUS. ROUTING on a bus is read-only
  (members routed into it, sends landing on it, output "Master") plus the
  `AUX RETURN` toggle.
- Master: CHAIN → AUTOMATION → MASTER (Bounce to WAV). No type tag.
- ROUTING on a track: audio input (audio tracks) or MIDI input
  (instrument/vocal), the output picker, and MIDI out for top-level MIDI
  tracks. An external-instrument track shows only the output picker; its
  pairing lives in TRACK.
- TRACK: Mono toggle, Bounce in place (instrument tracks, enabled per
  `classify_bounce`), and the external-hardware section.
- AUTOMATION: one row per lane on the channel (target name, Read toggle,
  remove) and a `+ Add lane` picker fed by the same option source as the
  strip's lane header (see [automation-lanes.md](../timeline/automation-lanes.md) for lanes).
- Every group folds independently (`MixerInspectorGroup`); TRACK, BUS and
  MASTER have separate keys. Fold state is runtime only, never saved.
- After every outermost update, `update::chain_ui::settle` drops the
  focused slot, ☰ menu, replace mode, preset prompt, drag, colour palette
  and picker cue once the inspector moves off their channel. A view switch
  (tab or Performance mode) commits an open rename and resets the CHAIN's
  transient state.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Mixer UI state | `resonance-app/src/state/mixer.rs` | `MixerUiState`, `MixerInspectorGroup`, `RenameState`, `ChainDragState`, `reset_for_project` |
| Layout | `resonance-app/src/view/mixer/mod.rs` | `view_mixer`, `view_track_cluster`, `view_add_bus_strip` |
| Strips | `resonance-app/src/view/mixer/{track_strip,bus_strip,master_strip,strip_parts}.rs` | `view_channel_strip`, `view_sub_channel_strip`, `view_bus_strip`, `view_master_strip` |
| Strip caching | `resonance-app/src/view/mixer/strip_fingerprint.rs` | per-strip fingerprints for the `lazy` body |
| Inspector | `resonance-app/src/view/mixer/inspector/` | `mod.rs::view`, `chain.rs`, `sends.rs`, `routing.rs`, `automation.rs`, `track_group.rs`, `bus.rs`, `master.rs` |
| Inline rename | `resonance-app/src/update/inline_rename.rs` | `UiMessage::BeginRename(RenameTarget, RenameSurface)` |
| Transient settle | `resonance-app/src/update/chain_ui.rs` | `settle`, `ChainUiMessage` |
| Widths/heights | `resonance-app/src/theme.rs` | `MIXER_STRIP_WIDTH`, `MIXER_SUB_STRIP_WIDTH`, `INSPECTOR_WIDTH`, `MIXER_STRIP_HEIGHT`, `BUS_STRIP_HEIGHT` |

Invariants:

- Each strip splits into a live fader/meter block, rebuilt every frame,
  and a `lazy` body keyed on its fingerprint. Anything the body draws must
  be hashed in `strip_fingerprint.rs` (or `inspector_fingerprint` for the
  inspector), or the view shows stale state.
- The focused slot always belongs to the channel the inspector describes.

## Surfaces

- **GUI:** Mix tab. `REF` chrome toggle for the reference rail.
- **Control API / MCP:** none of its own; the strip and inspector controls
  map onto the methods listed in the sibling feature docs. Renames:
  `track.rename` → `mcp__resonance__track_rename`, `bus.rename` →
  `mcp__resonance__bus_rename`.
- **Agent skills:** none (agents use the control API, not the view).

## How it's tested

| What | Where | Run |
|---|---|---|
| Strip anatomy, sub-track clustering | `resonance-app/tests/mixer/{mixer_strip_anatomy,mixer_sub_track_grouping,mixer_group_clustering}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Inspector groups, collapse, selection | `resonance-app/tests/mixer/{mixer_inspector_groups,mixer_inspector_collapse,mixer_selection_exclusivity,mixer_inspector_empty_project,mixer_transient_settle}.rs` | same |
| Lazy fingerprints | `resonance-app/tests/mixer/{inspector_lazy_fingerprint,tick_gating}.rs` | same |
| Inline rename | `resonance-app/tests/mixer/mixer_inline_rename.rs` | same |
| Goldens | `resonance-app/tests/snapshots/mixer_inspector_*-wgpu.png`, `mixer_rename_*-wgpu.png`, `mixer_sub_tracks_*-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer` |

Not covered: there is no golden of the whole Mix tab with track, bus and
master lanes together, and none of a bus or master strip on its own.

## Decisions and deviations

- The bottom plugin panel was removed; "open" always opens a window
  (in-app floating generic window or the plugin's own editor).
- The ☰ slot menu and the colour palette are inline cards under their
  row/header, not floating popovers.
- Bus strips have no solo; the spec drew one.
- Sub-track strips show slot lines (spec §2.4 listed none), because a
  sub-track can hold effects of its own.
- The AUTOMATION rows' "show in Arrange" jump was not built.

## Known gaps

- AUTOMATION rows have no jump to the lane in Arrange —
  [`../../intents/mixer/inspector-automation-jump.md`](../../intents/mixer/inspector-automation-jump.md).
- The sub-track strip's solo button changes nothing audible —
  [`../../intents/mixer/sub-track-solo.md`](../../intents/mixer/sub-track-solo.md).
- No sidechain controls anywhere in the inspector —
  [`../../intents/mixer/sidechain-gui.md`](../../intents/mixer/sidechain-gui.md).
