# Resonance UX Guidelines

These guidelines define the user experience principles and visual standards for Resonance. All UI work — in the main app (Iced) and plugin editors (egui) — must follow these rules.

## Design Philosophy

Resonance targets a **dark industrial aesthetic** — clean, minimal, and pro-audio focused. The UI should feel like hardware: every control has a clear purpose, nothing is decorative, and visual noise is kept to a minimum. The interface should disappear and let the user focus on music.

### Core Principles

1. **Clarity over density** — every element must be immediately readable at a glance. If a label or control isn't clear without a tooltip, it needs redesigning.
2. **Consistency** — identical actions look identical everywhere. A mute button in the mixer looks and behaves the same as in the track header.
3. **Direct manipulation** — prefer drag, click, and scroll interactions over menus and dialogs. The user should feel like they're touching the controls.
4. **Minimal chrome** — borders, separators, and containers should be subtle. Use spacing and background shade differences to group elements, not heavy outlines.
5. **Feedback** — every interaction must have visible feedback: hover states, press states, active indicators. No silent clicks.

## Color Palette

All colors are defined in `resonance-app/src/theme.rs`. Do not introduce ad-hoc colors.

| Role | Hex | Usage |
|------|-----|-------|
| BG (base) | `#0f0f0f` | Window background |
| Panel | `#1a1a1a` | Content panels, track headers |
| Panel Dark | `#141414` | Recessed areas, sub-panels |
| Separator | `#2a2a2a` | Borders, dividing lines |
| Accent | `#e8832a` | Active tabs, selection, highlights |
| Text | `#e0e0e0` | Primary text |
| Text Dim | `#808080` | Secondary text, labels, inactive |
| Record Red | `#cc3333` | Record arm, recording state |
| Solo Yellow | `#e6cc1a` | Solo state |
| Metronome Green | `#4acc4a` | Metronome on, active toggles |
| Loop Marker | `#e6b81a` | Loop region markers |
| Clip Body | `#4a7fa5` | Audio/MIDI clip fill |
| Clip Header | `#3a6f95` | Clip title bar |
| Meter BG | `#080808` | VU meter background |

### Color Rules

- Never use pure white (`#ffffff`) — the brightest text is `#e0e0e0`.
- Never use pure black (`#000000`) for backgrounds — the darkest is `#0f0f0f`.
- Accent orange is reserved for active/selected state. Do not use it for static decoration.
- Introduce new semantic colors only through `theme.rs`, never inline.

## Typography

- **UI text**: System default sans-serif via Iced's default font.
- **Icons**: Font Awesome Solid via the bundled "Resonance Icons" font (`theme::ICON_FONT`). Custom glyphs exist for metronome, mono/stereo indicators.
- **Sizing**: Use Iced's default text size as the baseline. Section headers may go 2-4px larger. Never go below 11px equivalent.
- **Shaping**: Use `Shaping::Basic` for icon text.

## Layout Constants

These are defined in `theme.rs` and must be used consistently:

| Constant | Value | Usage |
|----------|-------|-------|
| `TRACK_HEIGHT` | 80px | Arrange view track row height |
| `RULER_HEIGHT` | 30px | Timeline ruler height |
| `TRACK_HEADER_WIDTH` | 180px | Left-side track header panel |
| `MIXER_STRIP_WIDTH` | 160px | Mixer channel strip width |
| `MASTER_STRIP_WIDTH` | 140px | Master bus strip width |
| `FADER_HEIGHT` | 120px | Vertical fader travel |
| `PAN_KNOB_SIZE` | 28px | Pan knob diameter |
| `CLIP_EDGE_THRESHOLD` | 6px | Trim handle hit zone |

## Controls

### Buttons

Use the appropriate style function from `theme.rs`:

- `transport_button_style` — playback transport controls
- `record_armed_button_style` — record arm (red tint + border)
- `tab_button_style(active)` — view tab switching (Arrange/Mixer/Compose)
- `section_button_style(active, color)` — section buttons in Compose
- `toggle_button_style(active, color, small)` — toggles (monitor, metronome, punch)
- `mono_button_style(is_mono)` — mono/stereo toggle
- `small_button_style` — compact inline buttons (delete, add)
- `floating_button_style` — overlay buttons on canvas (zoom)

All buttons must have distinct hover, pressed, and default states. Rounded corners: 4px standard, 2px for small/inline buttons.

### Knobs

- Vertical drag to change value (not circular). Full travel = 140px.
- Shift held = fine adjustment.
- Double-click = reset to default/center.
- Arc indicator with dead zone at bottom (270-degree sweep).

### Faders

- Vertical orientation, 120px travel.
- Same shift-for-fine and double-click-reset as knobs.

### Metering

- VU meters use `METER_BG` (`#080808`) as background.
- Peak decay factor: 0.85 per frame tick (16ms).
- Peak hold indicator before decay.

## Interaction Patterns

### Mouse

- **Drag**: Primary interaction for knobs, faders, clip moving/trimming, selection.
- **Shift+drag**: Fine adjustment for continuous controls.
- **Double-click**: Reset to default (knobs, faders) or open edit mode (clip names, BPM).
- **Right-click**: Context menus where applicable.
- **Scroll**: Zoom (timeline), scroll (lists).

### Keyboard

- Transport controls should have single-key shortcuts (Space = play/stop, R = record).
- Tab-based navigation for main views.
- Escape to cancel/deselect.

## Container Hierarchy

Use the container style helpers from `theme.rs`:

1. `base_bg` — outermost window background
2. `panel_bg` / `panel_outlined` — main content areas (track headers, panels)
3. `panel_dark_bg` / `panel_dark_outlined` — recessed sub-areas (mixer strips)
4. `separator_bg` — 1px divider strips between regions
5. `timing_panel_style` — bordered panel for the transport timing display

Nesting depth should rarely exceed 3 levels. If you need more, reconsider the layout.

## Plugin Editors (egui)

Plugin UIs use egui via `wayland-plugin-gui`. They should feel consistent with the main app despite the different framework:

- Match the dark color palette as closely as possible.
- Use the shared plugin UI helpers from `resonance-plugin/src/ui.rs`.
- Tab-based layout for multi-section editors (OSC, ENV, FLT, LFO, MOD, FX).
- Parameter sliders should match the drag-to-adjust, shift-for-fine pattern.
- Visualization panels (oscilloscope, filter response, envelope shapes) go in a `viz/` submodule.

## Accessibility

- All interactive controls must have hover and active visual states — no invisible hit targets.
- Maintain sufficient contrast: text on panel backgrounds must be clearly readable.
- Don't rely on color alone to convey state — combine color with shape or position changes (e.g., record arm uses red color AND a border).

## Adding New UI Elements

When adding new controls or views:

1. Check if an existing control can be reused or extended.
2. Define any new colors in `theme.rs`, not inline.
3. Follow the existing message-driven architecture: UI emits `Message` variants, `update.rs` dispatches.
4. Keep view functions pure — no side effects, only return Iced elements.
5. New layout constants go in `theme.rs`.
6. Test at the standard window size and verify elements don't overlap or clip.
