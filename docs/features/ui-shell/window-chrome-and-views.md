# Window chrome, views and the remote indicator

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: none for the chrome as a whole. The remote indicator comes
> from ba doc #265 / todo #1159; the undo notice from code review UX-12.

The app has one window with four views: Arrange, Mixer, Compose and
Performance. Above the Arrange, Mixer and Compose views sit a 62 px chrome
row and a 74 px transport bar. The chrome shows:

- the project name and whether it is saved;
- the view tabs;
- view-specific toggles;
- Import and Settings;
- a green "Remote" chip while an agent or other control client is
  connected.

Performance mode replaces all of this with its own full-bleed shell
([performance-mode.md](performance-mode.md)).

## Behaviour

**Window.**

- Default and minimum size are both 1440×900
  (`state::DEFAULT_WINDOW_SIZE`).
- `exit_on_close_request: false`, so closing goes through the unsaved-changes
  flow ([overlays-and-dialogs.md](overlays-and-dialogs.md)).
- Antialiasing is off for frame cost.
- Fonts are bundled: Geist (UI), Geist Mono (readouts), Instrument Serif
  italic (titles, chord symbols) and a renamed Font Awesome ("Resonance
  Icons"). The default font is `theme::UI_FONT`.

**Views** (`ViewMode`).

- Switch with the tab buttons or ⌘1 / ⌘2 / ⌘3. F toggles Performance mode
  and Esc leaves it.
- `SwitchView` remembers the view Performance was entered from
  (`ui.pre_performance_view`) and goes back to it.
- Leaving a view commits any inline rename and closes the mixer CHAIN's
  transient state (`leave_view`).
- Switching views never touches the transport.
- The binary takes `--tab arrange|mixer|compose|performance` (also
  `--tab=…`), parsed by `parse_startup_tab` into `STARTUP_TAB`. Only the
  real machine host reads it; test apps start on Arrange unless they use
  `new_for_test_on(tab)`.

**Chrome row** (`view/transport.rs::view_chrome`), left to right:

1. Brand dot and "Resonance", "/", then the project file stem in serif
   italic ("Untitled" when there is none).
2. "· not saved" (untitled), "· unsaved" (dirty) or "· saved".
3. A fixed 200 px slot for the undo/redo notice.
4. The remote indicator.
5. The centred segmented tabs, each with a registry tooltip.
6. Arrange only: **Media** (toggles the docked media browser,
   [media-browser-panel.md](media-browser-panel.md)) and **Import audio…**.
7. Mixer only: **REF** (the Reference & A/B rail,
   [reference-ab-monitoring.md](../mixer/reference-ab-monitoring.md)).
8. **Import…** (the MIDI import modal) and the ☰ **Settings** button.

Elements that come and go are replaced by zero-width spacers, so the row
never reflows.

**Transport bar** (`view_playback_bar`). It holds:

- the transport buttons (skip, stop, play/pause, record, loop, metronome,
  prev/next marker, markers overview), each with a registry tooltip;
- the BPM, signature and position readouts;
- the CPU readout;
- the master meter (a Canvas).

What the transport does is documented in [transport.md](../hardware-io/transport.md) and
[engine-command-loop.md](../audio-engine/engine-command-loop.md).

**Status area.** Between the transport and the main area, see
[overlays-and-dialogs.md](overlays-and-dialogs.md).

**Arrange main area** (`view_main_area`). It holds:

- the optional media browser, then the track headers and the timeline;
- the MIDI editor panel docked below, when a MIDI clip is open;
- the clip fade/gain inspector, floating top right;
- the track context menu, stacked in arrange-area coordinates.

**Remote indicator** (`view/remote_indicator.rs`).

- It is hidden (a zero-width `Space`) while
  `Resonance::control_client_count()` is 0. That count is the number of
  open control-socket sessions (`control.sessions`).
- Otherwise it shows a `GOOD` dot and "Remote" on a `GOOD_DIM` pill, with
  the tooltip "N remote control client(s) connected".
- It is the only visible GUI of the control API. Every agent edit also
  shows up live in the views and in undo.

**Drops on the window.**

- Dropping a `.mid` / `.midi` file opens the MIDI import modal.
- In Arrange, dropping audio files imports them onto a new audio track at
  the playhead.
- In Mixer, dropping audio files loads them as references.

These listeners are attached per view, so a stray drop in another view does
nothing.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Window and fonts | `resonance-app/src/main.rs`, `resonance-app/src/theme.rs` | `iced::application`, `UI_FONT_FACES`, `ICON_FONT_BYTES`, `resonance_theme` |
| View mode | `resonance-app/src/state/viewport.rs`, `update/ui.rs` | `ViewMode`, `SwitchView`, `set_view`, `leave_view`, `toggle_performance_mode` |
| CLI tab | `resonance-app/src/lib.rs` | `parse_startup_tab`, `STARTUP_TAB` |
| Root view | `resonance-app/src/view/mod.rs` | `view`, `view_base`, `view_main_area` |
| Chrome and transport bar | `resonance-app/src/view/transport.rs` | `view_transport`, `view_chrome`, `tab_button`, `view_playback_bar`, `CHROME_HEIGHT`, `TRANSPORT_HEIGHT` |
| Remote chip | `resonance-app/src/view/remote_indicator.rs`, `lib.rs` | `view(client_count)`, `control_client_count` |
| Window events | `resonance-app/src/update.rs` | `subscription` (close requests, file drops, resize) |

## Surfaces

- **GUI:** as above.
- **Control API / MCP:** there is no method to switch views. Any connected
  client lights the Remote chip.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Remote chip shown and hidden | `resonance-app/tests/performance/remote_indicator_snapshot.rs` | `./scripts/run-tests.py -p resonance-app` |
| Tab and Performance switching, return view | `resonance-app/tests/performance/performance_mode_toggle.rs` | same |
| Untitled "not saved" label | `resonance-app/tests/io/project_lifecycle_ux.rs` | same |
| CPU readout and undo notice in the chrome | `resonance-app/tests/io/project_lifecycle_ux.rs` (golden `transport_cpu_warm_undo_notice-wgpu.png`) | same |
| Goldens | `remote_indicator_active`, `untitled_chrome_not_saved`, `audition_transport_*` in `resonance-app/tests/snapshots/` | same |

Not covered: `parse_startup_tab` (it reads `std::env::args`) and the
window-level file-drop subscriptions. Tests send the resulting messages
directly.

## Decisions and deviations

- There is no menu bar. The command palette replaces it (command-palette
  spec §2).
- The remote chip only shows that a client is connected. It does not show
  what the client is doing.
- The settings view's module comment says the gear icon opens it. The chrome
  button is actually a ☰ (`fa::BARS`) icon.

## Known gaps

- There is no OS-level fullscreen. Performance mode is an in-window view.
