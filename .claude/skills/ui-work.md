# UI Work

This skill should be used when the user asks to "add a button", "tweak the transport", "change the mixer layout", "add an icon", "restyle X", "fix alignment", "add a dialog", "make X floating", "add a scrollbar", or otherwise wants to modify the Resonance application's iced-based desktop UI. It also applies when designing a new panel, overlay, or interactive canvas widget.

## Overview

Resonance's GUI lives in `resonance-app/` and is built with **iced 0.13** using the `wgpu` + `canvas` + `tokio` feature set. The app is a single `iced::application` with a `Resonance` state struct, an `update(&mut self, Message)` reducer, and a `view(&self) -> Element<Message>` tree. There are no child components — the entire UI is pure functions of state.

There are three visual idioms in use:

1. **Widget trees** (`view/mod.rs`, `view/mixer.rs`) — composed from iced's `row`/`column`/`container`/`button`/`text` widgets.
2. **Canvas programs** (`timeline.rs`, `midi_editor.rs`) — custom drawing + mouse/keyboard handling via `canvas::Program<Message>`.
3. **Stacked overlays** — settings dialog, add-track menu, floating zoom buttons, all via `stack![base, overlay]`.

Any real UI work will touch the message enum, the state struct, the view tree, and usually the theme module.

## Critical files

```
resonance-app/
  assets/fonts/fa-solid-900.otf   ← custom icon font (Resonance Icons)
  src/
    main.rs             ← Resonance state struct + iced::application bootstrap
    message.rs          ← Message enum (one variant per user action / engine event)
    update.rs           ← update() reducer + Tick/keyboard subscription
    view/
      mod.rs            ← top-level view(), transport bar, settings dialog, track headers
      mixer.rs          ← mixer view
    theme.rs            ← colors, button styles, container styles, ICON_FONT, fa::* codepoints
    timeline.rs         ← TimelineCanvas canvas::Program + scrollbars
    midi_editor.rs      ← PianoRollCanvas canvas::Program
    state.rs            ← track/clip/drag state types
  tools/
    add_metronome_glyph.py  ← fonttools script for extending the icon font
```

## Step-by-Step: Common UI Tasks

### 1. Use the custom icon font — never raw Unicode emoji

The app ships with a **custom icon font** at `resonance-app/assets/fonts/fa-solid-900.otf`. It is a modified copy of Font Awesome 7 Free Solid with:

- A renamed family (`"Resonance Icons"`, PostScript name `ResonanceIcons-Solid`) so it cannot collide with any system-installed Font Awesome.
- An extra custom `metronome` glyph at `U+F8DB` (the Pro-tier FA icon is not in the Free font).

It is loaded once in `main.rs` via `.font(theme::ICON_FONT_BYTES)` and the `theme::ICON_FONT` constant references it by family name.

**Always render icons through `theme::icon(char)`**. Never use `text("\u{...}").shaping(Shaping::Advanced)` — that path is unreliable because it falls through to system font matching.

```rust
use crate::theme::{self, fa};

let play_btn = button(theme::icon(fa::PLAY).size(16).color(theme::ACCENT))
    .on_press(Message::Play)
    .padding([6, 10])
    .style(|_theme, status| theme::transport_button_style(status));
```

**Available icon codepoints** (in `theme::fa`):

| Constant | Glyph | Use for |
|---|---|---|
| `PLAY`, `PAUSE`, `STOP` | ▶ ⏸ ■ | Transport playback |
| `BACKWARD_STEP`, `FORWARD_STEP` | ⏮ ⏭ | Skip back / forward |
| `CIRCLE` | ● | Record button |
| `BARS` | ☰ | Settings / menu |
| `FOLDER_OPEN`, `FLOPPY_DISK` | 📁 💾 | Open / save |
| `MAGNIFYING_GLASS_PLUS/MINUS` | 🔍 | Zoom in / out |
| `METRONOME` | (custom) | Metronome toggle |
| `BULLSEYE` | ◎ | Punch-in/out |

If you need a glyph that isn't listed: first check whether Font Awesome 7 Free Solid already contains it. Browse https://fontawesome.com/search?o=r&m=free&s=solid and verify the codepoint by reading the Unicode value from the icon's page. If the font already has it, just add a `pub const FOO: char = '\u{...}'` to `theme::fa`. If the icon is Pro-only or doesn't exist, you must draw it and inject it into the font — see "Adding a custom glyph" below.

### 2. Adding a custom glyph to the icon font

Do this only if Font Awesome Free doesn't have the icon you need.

Use the existing `tools/add_metronome_glyph.py` as a template — it uses `fontTools.pens.t2CharStringPen.T2CharStringPen` to draw outlines directly into the CFF charstring table.

**Coordinate system**:
- `unitsPerEm = 512`, `ascent = 448`, `descent = -64`.
- Font uses **y-up**.
- **Every FA Solid icon in this font is vertically centered on y=192** (the midpoint of ascent and descent). Non-centered glyphs render visibly offset from surrounding icons — you MUST match this center or alignment will drift.
- Typical vertical span is ~448 units (e.g. `play`/`stop` go -32..416). Horizontal span ~416 units wide.
- Use counter-clockwise winding for outer contours; CFF uses nonzero winding fill.

**Workflow**:

1. Make sure the fonttools venv exists: `python3 -m venv /tmp/fontvenv && /tmp/fontvenv/bin/pip install fonttools`
2. Copy `tools/add_metronome_glyph.py` to a new file, rename the glyph and edit `draw_metronome` to produce your new shape. Pick an unused codepoint in the Private Use Area (e.g. `U+F8F0`+) or reuse an unmapped FA Pro codepoint.
3. **Inspect an existing glyph for reference bounds**:
   ```bash
   /tmp/fontvenv/bin/python3 -c "
   from fontTools.ttLib import TTFont
   from fontTools.pens.boundsPen import BoundsPen
   f = TTFont('resonance-app/assets/fonts/fa-solid-900.otf')
   gs = f['CFF '].cff.topDictIndex[0].CharStrings
   for name in ('play', 'stop', 'bars', 'circle'):
       pen = BoundsPen(glyphSet=None)
       gs[name].draw(pen)
       print(name, pen.bounds, 'center_y:', (pen.bounds[1]+pen.bounds[3])/2)
   "
   ```
4. Run your script: `/tmp/fontvenv/bin/python3 tools/your_script.py`.
5. **Verify bounds match the y=192 center** by running the inspect snippet on your new glyph.
6. Add the codepoint to `theme::fa` and wire it into the view.
7. `cargo run -p resonance-app` to test. If the icon appears as a blank square, iced is picking up a different font copy — check that the bundled font's family name is unique (should be `"Resonance Icons"`, not `"Font Awesome ..."`).

**Do not modify the font family name** — it must stay `"Resonance Icons"` so it never clashes with system-installed fonts. The rename is handled automatically by the script's `rename_font()` helper.

### 3. Adding or changing a button in the transport / mixer / panel

The canonical button pattern:

```rust
let my_btn = button(theme::icon(fa::SOMETHING).size(16).color(theme::TEXT))
    .on_press(Message::DoThing)
    .padding([6, 10])                     // transport buttons: [6, 10]
    .style(|_theme, status| theme::transport_button_style(status));
```

For **toggles** (metronome, monitor, punch) where the button has an active color, use `theme::toggle_button_style(enabled, active_color, small, status)`:

```rust
let enabled = self.my_thing_enabled;
let btn = button(theme::icon(fa::FOO).size(16).color(
    if enabled { theme::METRONOME_ON } else { theme::TEXT_DIM },
))
.on_press(Message::ToggleFoo)
.padding([6, 10])
.style(move |_theme, status| {
    theme::toggle_button_style(enabled, theme::METRONOME_ON, false, status)
});
```

For **disabled** buttons (e.g. record when no track is armed): omit `.on_press()`. iced renders buttons with no press handler as non-interactive automatically. Do not add an `AskUserQuestion`-style "please arm a track" popup unless the user asks — silent disabled-and-dim is the house style.

**Button style helpers** in `theme.rs`:
- `transport_button_style(status)` — bordered, filled background, the default for any "real" button.
- `small_button_style(status)` — transparent background, no border. For inline controls on track headers and mixer strips.
- `record_armed_button_style(status)` — red-tinted variant, used when a record-arm button is active.
- `tab_button_style(active, status)` — used for the arrange/mixer view tab switcher.
- `floating_button_style(status)` — semi-opaque, for overlays that sit on top of a canvas (e.g. zoom buttons on the timeline).
- `toggle_button_style(active, color, small, status)` — generic on/off toggle.

**Never inline-define a button style** unless it's genuinely one-off (e.g. the punch button's amber state). Prefer adding a helper to `theme.rs` so the look stays consistent.

### 4. Laying out rows where text is mixed with icons

This is the single most common source of alignment bugs in the app. iced text widgets use the source font's `hhea` line metrics, so a row that mixes `Font::MONOSPACE` values with `theme::ICON_FONT` values will render the icons at a subtly different vertical position than the digits next to them, because the two fonts have different ascent/descent ratios.

**Rules for a cleanly-aligned mixed row**:

1. **Pin `line_height(1.0)` on every text in the row**:
   ```rust
   let tight = iced::widget::text::LineHeight::Relative(1.0);
   let t = text("120").size(18).line_height(tight).font(Font::MONOSPACE);
   ```
   This forces each text's layout box to equal its font size, so both fonts produce the same box height.

2. **Wrap each cell in a fixed-height container with explicit centering**. Use local helpers — see `view_transport` in `view/mod.rs` for the reference `value_cell()` / `label_cell()` pattern:
   ```rust
   fn value_cell<'a>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
       container(content)
           .width(Length::Fill)
           .height(22.0)
           .align_x(alignment::Horizontal::Center)
           .align_y(alignment::Vertical::Center)
   }
   ```

3. **Use `mouse_area`, not `button`, for clickable cells inside a tight grid**. A `button` injects layout padding that is hard to fully zero out, shifting its content relative to plain-text siblings. `mouse_area` is a bare event wrapper with zero layout impact:
   ```rust
   let time_sig = mouse_area(
       text("4/4").size(18).line_height(tight).font(Font::MONOSPACE)
   ).on_press(Message::CycleTimeSignature);
   ```
   Use `button` only where you actually want a button's visual affordance (background, border, hover highlight).

4. **When drawing a new icon glyph, always match the project's y=192 center** (see §2). A glyph centered at, say, y=224 will sit higher than the digits next to it even with perfect layout code.

### 5. Adding a modal dialog / popup

Modals go above the base content via `stack![base, overlay]`. See `view_settings_overlay()` and `view_add_track_menu()` in `view/mod.rs` for reference.

Skeleton:

```rust
fn view_my_modal(&self) -> Element<'_, Message> {
    let backdrop = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(
                    iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6),
                )),
                ..Default::default()
            }),
    )
    .on_press(Message::CloseMyModal);

    let dialog_content = column![/* ... */]
        .spacing(6)
        .padding(24)
        .width(420);

    let dialog = container(opaque(dialog_content))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });

    let centered = container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}
```

**Important**:
- Wrap the dialog content in `opaque(...)` so click-through to the backdrop is blocked on the dialog surface.
- The backdrop `mouse_area` must be the first stack layer — its `on_press` closes the dialog when the user clicks outside.
- Add a `self.my_modal_open: bool` field to `Resonance` in `main.rs` and branch in the top-level `view()`:
  ```rust
  if self.settings_open { stack![base, self.view_settings_overlay()].into() }
  else if self.my_modal_open { stack![base, self.view_my_modal()].into() }
  else { base }
  ```

### 6. Floating controls on top of a canvas

Floating overlays (e.g. zoom buttons on the timeline) use `stack![canvas_el, overlay]` where the overlay is a full-size transparent container that shrink-wraps only the buttons:

```rust
let overlay = container(
    column![
        Space::with_height(Length::Fill),               // push buttons to bottom
        row![Space::with_width(Length::Fill), buttons], // push buttons to right
    ],
)
.width(Length::Fill)
.height(Length::Fill)
.padding(iced::Padding { top: 0.0, right: 20.0, bottom: 20.0, left: 0.0 });

stack![canvas_el, overlay].into()
```

**Critical**: the wrapping container must have no background and must only size the button group to its natural size. The `Space::with_*(Length::Fill)` push the buttons into position, and only the button rects hit-test for clicks — everything else passes through to the canvas below.

Use `theme::floating_button_style` for the buttons so they're semi-opaque and read clearly against clip content.

### 7. Adding a new Message

1. Add a variant to `Message` in `message.rs`. Use concise, imperative names (`Play`, `CycleTimeSignature`, `SetBpmText(String)`).
2. Handle it in `update.rs`. Typically: mutate `self`, and/or send an `AudioCommand` via `self.engine.send(...)`, and/or return an `iced::Task` for async work.
3. Wire it up in the view with `.on_press(Message::Foo)`.

**State that lives on the UI side** (mute state, selected clip, scroll offsets, dialogs): owned by `Resonance` in `main.rs`, mutated directly in `update.rs`.

**State that lives in the audio engine** (playhead, recording state, clip buffers): **never directly mutated** from `update.rs`. The UI sends an `AudioCommand` and waits for an `AudioEvent` in `engine_events.rs` that reflects the committed change. Follow the existing `Message::Play` → `AudioCommand::Play` → `AudioEvent::Stopped/RecordingStarted/...` pattern.

### 8. Canvas widgets (timeline / piano roll)

Canvas programs implement `iced::widget::canvas::Program<Message>`. Reference: `timeline.rs::TimelineCanvas` (with scrollbars) and `midi_editor.rs::PianoRollCanvas`.

A canvas program has:
- Immutable `&self` data passed in from `view()` each frame (tracks, clips, scroll offsets, zoom).
- A `State` associated type for per-canvas UI state that persists across frames (active drags, last click timestamp, cached viewport width).
- `update()` — receives mouse/keyboard events, may return a `Message` to dispatch to the app reducer.
- `draw()` — returns one or more `canvas::Geometry`s to render.

**Canvas best practices**:

- **Report layout info back to the app reducer** rather than trying to clamp scroll offsets inside the canvas. See how `TimelineCanvas` fires `Message::ViewportWidth` and `Message::TimelineContentSize` at the end of `update()` when those values change. The app then clamps `scroll_offset` in `update.rs::Message::ScrollX`.
- **Draw scrollbars last**, inside the canvas. iced doesn't give you real scrollbars on a canvas; fake them by computing a thumb rect from `(scroll_offset, content_width, viewport_width)` and drawing a thin strip along the bottom/right edge. Hit-test for mouse events in `update()` before any clip hit-testing so scrollbar clicks aren't consumed by the content layer. Hide the scrollbar entirely when `content_width <= viewport_width`.
- **Guard `position_in(bounds)` checks** before handling wheel events. Without this, scrolling one canvas will also scroll a canvas behind/beside it, which happened with the piano roll overlaying the timeline.
- Use theme constants (`theme::TRACK_HEIGHT`, `theme::RULER_HEIGHT`, `theme::TRACK_HEADER_WIDTH`) rather than hard-coding sizes. If you need a new constant, add it to `theme.rs`.

### 9. Colors — always from `theme.rs`

Never use `iced::Color::from_rgb(...)` inline for anything that belongs to the app's palette. Add a constant to `theme.rs` first, then reference it. Current palette:

- `BG`, `PANEL`, `PANEL_DARK`, `PANEL_ARMED` — surface hierarchy
- `SEPARATOR`, `TRACK_LINE`, `BAR_LINE`, `BEAT_LINE` — rules & grid
- `TEXT`, `TEXT_DIM` — foreground text
- `ACCENT` — primary interaction color (orange)
- `RECORD_RED`, `SOLO_YELLOW`, `METRONOME_ON`, `LOOP_MARKER` — semantic
- `CLIP_BODY`, `CLIP_HEADER`, `CLIP_SELECTED_BORDER` — clip rendering
- `RULER_BG` — timeline ruler background

### 10. Verifying your changes — screenshot loop

After any UI work, you must **visually verify** the result, not just
type-check it. The app supports a `--demo` flag that boots into a fully
seeded project (tracks, busses, clips, compose section) so views render
with realistic content without a real audio engine, and `--tab
arrange|mixer|compose` selects which view opens first. Combine those
with `spectacle` (KDE's window grabber, already installed) and
ImageMagick `magick` to capture a screenshot you can read with the Read
tool.

**Canonical recipe** — adapt the `--tab` value and crop region to the
view you changed:

```bash
mkdir -p /tmp/resonance-shots
./target/debug/resonance-app --tab mixer --demo >/dev/null 2>&1 &
APP_PID=$!
sleep 4                                # let wgpu/init settle before grabbing
spectacle --activewindow --background \
  --output /tmp/resonance-shots/mixer.png --nonotify >/dev/null 2>&1
sleep 1
kill -TERM $APP_PID 2>/dev/null
wait $APP_PID 2>/dev/null
magick /tmp/resonance-shots/mixer.png -resize 1500 \
  /tmp/resonance-shots/mixer-display.png
```

Then **read the PNG with the Read tool** — Claude Code renders the image
inline so you can inspect alignment, spacing, color, and overlap with
your own eyes. Don't skip this step; type-checking alone has no opinion
on whether a strip is the right height or whether two elements overlap.

**Cropping for detail.** When a small region matters (a single mixer
strip, the transport bar, a track header), follow the screenshot with a
crop so the detail isn't lost in the resize:

```bash
magick /tmp/resonance-shots/mixer.png -crop 800x500+200+200 \
  /tmp/resonance-shots/strip-detail.png
```

The geometry is `WIDTHxHEIGHT+X+Y` in source-image pixels (before
resize). Skip the `-resize` step on detail crops so you see the native
resolution.

**Multi-tab checks.** If a change touches shared chrome (toolbar,
transport, theme), capture all three tabs in the same loop — replace
`mixer` with `arrange` / `compose` and rerun. Each tab is a separate
process invocation; don't try to switch tabs inside a single run.

After the screenshot pass:

1. `cargo build -p resonance-app` — must be warning-free for code you touched.
2. Screenshot the affected view(s) per the recipe above and read the
   image. Confirm alignment, spacing, hover/pressed/disabled states
   (use `--tab` plus the demo seed to land on a state that exercises
   them — e.g. armed tracks are pre-seeded), and behaviour when the
   underlying state is empty.
3. If you added/changed an icon glyph, re-verify the glyph bounds match
   the y=192 project center (see §2) — and screenshot the row that uses
   it to confirm vertical alignment with neighbouring text.
4. If you added a scrollable/scrollbarred area, confirm the scrollbar
   hides when content fits and that wheel/drag both work.

## Best Practices Summary

- **Icons**: `theme::icon(fa::X)`, never raw `text("\u{...}")`.
- **Buttons**: use theme style helpers; only inline a style for truly one-off looks.
- **Text in mixed-font rows**: `line_height(Relative(1.0))` + fixed-height centered container, every time.
- **Clickable cells in tight grids**: `mouse_area`, not `button`.
- **Colors**: from `theme.rs`, never inline RGB.
- **State**: UI-only state on `Resonance` in `main.rs`; engine state round-trips through `AudioCommand`/`AudioEvent`.
- **Modals**: `stack![base, overlay]` with backdrop `mouse_area` and `opaque` dialog.
- **Floating overlays**: full-size transparent wrapper pushing a shrunk button group; no background.
- **Canvas scroll**: report `ViewportWidth` + `TimelineContentSize` back to the app, clamp in the reducer.
- **New custom glyphs**: center on y=192, counter-clockwise winding, match ~448-unit vertical span, rename never — only `tools/add_metronome_glyph.py`-style scripts touch the font.
- **Disabled controls**: drop `.on_press(...)` and dim the text color. No popups asking "please do X first".
- **Don't duplicate master controls across tabs**: the mixer owns the master fader; the transport must not. Analogous: zoom lives on the timeline, not the transport.
- **Always screenshot-verify**: never report UI work complete on a `cargo check` alone — boot with `--demo --tab <view>`, grab with `spectacle --activewindow`, read the PNG. See §10.
- **Per-frame allocations are the silent perf killer**: iced rebuilds the entire widget tree every frame. See §11 for the rules — cached pick-list options on `view_caches`, fixed-height strips, canvas-cached meters, and `iced::widget::lazy` for non-live regions.

## 11. View performance — what to do (and not do) inside `view()`

Resonance's iced GUI rebuilds the entire widget tree per frame. During
a continuous window resize that's 60+ rebuilds/sec, so anything
allocated inside `view()` allocates that often. The rules below are
the difference between a smooth resize and a chunky one.

### 11.1 Cache pick_list options on `view_caches`

`pick_list(options, …)` takes the options by value. Building a fresh
`Vec` each frame for device lists, plugin lists, or bus-output
destinations is the single biggest source of resize jank we've found.

The canonical home for these is
`resonance-app/src/view/ui_caches.rs::UiViewCaches`. Each cached list
is an `Rc<[T]>` — view code clones the Rc (refcount bump only) and
passes it to `pick_list`. Rebuild only happens when source data
changes (device hot-plug, plugin scan, bus add/remove), driven from
the engine event handler, e.g.:

```rust
pub(super) fn midi_input_devices(r: &mut Resonance, devices: Vec<MidiDeviceInfo>) {
    r.midi_input_devices = devices;
    r.view_caches.rebuild_midi_input(&r.midi_input_devices);
}
```

If you add a new pick_list whose options are a function of state that
doesn't change every frame, add a cache field to `UiViewCaches` plus a
`rebuild_*` method, and wire the rebuild into the relevant handler(s).
Don't filter+clone inside `view()`.

For the rare case where the cached list needs a per-call override
entry (e.g. a stale-but-still-configured MIDI device), wrap it in
`ChoiceList<T>` (in `ui_caches.rs`) so the cached and owned variants
both implement `Borrow<[T]>` and slot into `pick_list` identically.

### 11.2 Lazy-wrap regions that don't update per-tick

`iced::widget::lazy(dep_hash, |_| build(...))` caches the produced
widget tree across frames and only re-runs the closure when `dep_hash`
changes. Resize doesn't change state, so the cache hits across every
paint during a resize.

The output of the closure must convert to `Element<'static>` — so the
build function must only own data it puts into widgets (no `&str`
slices borrowed from `&Resonance`, no `Element<'a, …>`). Practically,
update helper signatures from `fn foo<'a>(r: &'a Resonance) -> Element<'a, Message>`
to `fn foo(r: &Resonance) -> Element<'static, Message>` and let the
compiler tell you where a borrow leaks through.

**Caveat — never put a `canvas::Cache`-backed widget inside a lazy
block whose dep hash skips that widget's live data.** The lazy widget
stores the cached `Element` (which contains the Canvas program with
its level fields), and the cached Canvas program is reused frame
after frame with frozen field values. The meter widget in
`view/controls.rs` is the obvious example — wrapping a strip with
meters in lazy freezes the meter. Either keep the meter outside the
lazy area, or keep the entire strip outside lazy.

Reference implementations: `view/mixer/inspector.rs::view` wraps the
routing + chain groups in `lazy` keyed on `inspector_fingerprint`;
`view/track_header.rs::view_track_headers` wraps the whole arrange
header column in `lazy` keyed on `track_headers_fingerprint`. Both
hash every visible field except levels, so lazy invalidation tracks
user input + engine events rather than per-tick meter updates.

### 11.3 Cache the sort, not the sort key

`TrackRegistry::tracks` and `.busses` are kept sorted by `.order` as
an invariant — view code calls `sorted_tracks() / sorted_busses()`
which iterates them as-is, no per-frame `sort_by_key`. Any mutation
that changes `.order` (re-ordering, certain track-move flows) must
call `registry.resort_tracks()` / `resort_busses()` afterwards. Push
and `retain` preserve order so they don't need a resort.

A debug-only assertion at the top of `sorted_tracks` panics if the
invariant breaks, so violations show up immediately under `cargo run`.

### 11.4 Render meters as Canvas widgets

`view::controls::meter_v` builds a `StereoMeterCanvas` with a
`canvas::Cache` keyed on the bit pattern of the two level values.
Hover and resize redraws hit the cache; only a real level change
invalidates. Use this pattern any time you need a value-driven visual
that updates faster than user input — replicate `StereoMeterCanvas` /
`KnobState` rather than building a column of resized containers.

### 11.5 Fixed-height strips, not Length::Fill chains

The mixer strip card has a fixed `MIXER_STRIP_HEIGHT` (and busses use
`BUS_STRIP_HEIGHT`). The FX list inside scrolls. Avoid `Length::Fill`
on every nested container — it forces iced to recompute child layouts
every time the parent resizes, which is the visible cause of "the
mixer feels sluggish during resize." See `view::mixer::track_strip`
for the pattern: fixed-height outer container, scrollable FX list,
fader pinned at the bottom.

The same pattern applies to **content canvases that own their own
scroll state**: prefer a fixed-pixel-width canvas inside a horizontal
`Scrollable` over a `Length::Fill` canvas with internal `scroll_x`.
Two reasons: (a) the canvas's `canvas::Cache` keys on `bounds.size()`,
so a Fill canvas invalidates the cache on every paint of a continuous
resize; a fixed-width canvas hits the cache. (b) Notes / clips stay
at a stable pixel size instead of being squashed or stretched as the
window resizes. The piano roll
(`view::view_midi_editor_panel`) and the arrange timeline
(`view::view_timeline`) both follow this pattern — canvas width is
the natural content width plus a small trailing pad, wrapped in a
horizontal `Scrollable`, and the canvas's wheel-X handler returns
`Ignored` so the scrollable receives those events instead.

**Vertical scroll can stay internal** when the canvas has tightly
coupled header rows that must scroll in lockstep with the body
(ruler + section band + global-tracks header on the timeline; the
keyboard column on the piano roll). The timeline does this — the
in-canvas vertical scrollbar lives on, the horizontal one was deleted
when its handler became dead code. Don't try to put a canvas with
sticky headers in a `Direction::Both` scrollable unless you're also
prepared to extract the header into a separate widget.

### 11.6 What we deliberately did NOT do

`Arc<str>` for `TrackState.name` / `BusState.name` / `PluginSlotState.plugin_name`
was considered. After lazy lands, the widget tree rebuilds only on
state change (~20Hz max), so per-rebuild `String` allocations are
negligible. Re-evaluate only if profiling under load points at name
cloning. Don't do the wide refactor pre-emptively.
