# Handoff: Resonance — DAW Redesign

## Overview
Redesign of **Resonance**, a desktop digital audio workstation. The brief was a lighter, less dense, more modern look — a softer dark theme with breathing room, color-coded affordances, and a shared visual system across the three top-level views: **Arrange**, **Mixer**, and **Compose**.

## About the Design Files
The HTML/JSX in `design/` are **design references**. They are React + Babel prototypes that render in a single HTML page (`Resonance.html`) — not production code. Your job is to **recreate these designs in the Resonance codebase using its existing framework, components, and patterns**. If there is no current frontend (e.g. it's Qt/native), translate the visual system, layout, and interactions into the target framework idiomatically.

The prototypes are split per view to mirror how the production code is likely organized:
- `app.jsx` — window chrome, top tabs, transport bar (shared)
- `arrange.jsx` — Arrange view
- `mixer.jsx` — Mixer view
- `compose.jsx` — Compose view
- `main.jsx` — view switching + Tweaks panel (theme/density)
- `tweaks-panel.jsx` — utility panel for runtime tweaks (not part of production)

## Fidelity
**High-fidelity.** Colors, type scale, spacing, radii, and component anatomy are intentional. Hover and selection states are wired in. Match exactly unless your existing system provides closer equivalents — in that case prefer the existing token.

---

## Design Tokens

### Color
| Token | Hex | Use |
|---|---|---|
| `--bg-0` | `#0f1013` | Page / window backdrop |
| `--bg-1` | `#15161b` | App body |
| `--bg-2` | `#1b1d23` | Panels, channel strips, cards |
| `--bg-3` | `#23262e` | Hover, raised controls |
| `--bg-4` | `#2c2f38` | Fader caps, knob highlights |
| `--line` | `#272a31` | Standard borders |
| `--line-2` | `#1f2229` | Subtle dividers / hairlines |
| `--text-1` | `#e8e7e3` | Primary text |
| `--text-2` | `#9aa0ac` | Secondary text |
| `--text-3` | `#5d626d` | Tertiary / labels |
| `--text-4` | `#3f434c` | Disabled |
| `--accent` | `#8b6dff` | Lavender — primary brand / selection |
| `--accent-soft` | `#a892ff` | Lighter lavender — text on dim bg |
| `--accent-dim` | `rgba(139,109,255,.16)` | Selection wash |
| `--accent-line` | `rgba(139,109,255,.34)` | Selection border |
| `--warm` | `#e8c47b` | Audio clips, busses, playhead |
| `--good` | `#6dd6a3` | Meters, success |
| `--bad` | `#e87b8b` | Mute, peaking, errors |

The accent is user-tweakable via the Tweaks panel — supported swatches are `#8b6dff` (lavender), `#e8c47b` (amber), `#6dd6a3` (mint), `#e8e7e3` (cream/monochrome). The system derives soft / dim / line variants programmatically (see `lighten()` and `hexA()` in `main.jsx`).

### Typography
- **UI sans:** `Geist` (Google Fonts) — weights 300/400/500/600/700
- **Mono:** `Geist Mono` — weights 400/500/600 — used for numbers (BPM, dB, bar counts, seeds)
- **Display:** `Instrument Serif` italic — used sparingly for the project title in the chrome and chord symbols

Sizes:
- 22 / italic serif — chord symbols
- 17 — BPM display, inspector titles
- 13 — track names, panel titles
- 12 — body, buttons
- 11 — track meta, control labels
- 10 / 10.5 — secondary labels
- 9 / 9.5 — uppercase letterspaced labels (`SCALE`, `OUT`, `dB`)

Letter-spacing: uppercase labels use `.14em`–`.18em`.

### Spacing
- 4 / 6 / 8 / 10 / 12 / 14 / 16 / 18 base scale
- Card padding: 10–14
- Panel padding: 16–18
- Grid gaps: 6 (cells), 10–12 (cards), 16 (sections)

### Radius
- `4` cells / tiny buttons
- `6` segmented tabs
- `7` standard buttons & inputs
- `8–9` clip cards, chord cards, instrument slots
- `12` strip cards, drum grid panel
- `14` outer window

### Shadows
Used sparingly. Strip selection: `0 0 0 3px rgba(139,109,255,.10)`. Window: `0 20px 60px rgba(0,0,0,.6), 0 0 0 1px rgba(255,255,255,.04)`.

---

## Window Chrome (shared)
Top bar, 48px tall, `--bg-1` background.
- **Left:** macOS-style traffic-light dots (red/yellow/green, 11px), then `● Resonance` brand mark + `/` + project title in italic serif + “· edited 2m ago” in muted text.
- **Center:** segmented tab control with `Arrange`, `Mixer`, `Compose`. Active tab gets `--bg-3` background and `--text-1` color.
- **Right:** ghost ⌘K search button, ghost Share button, gradient lavender avatar.

## Transport Bar (shared)
56px tall, below chrome.
- **Left:** transport buttons (prev / stop / play / record / next), then divider, loop, metronome. Play button is a 36×36 lavender pill (background `--accent`, dark text); other buttons are 32×32 with `--bg-2` and `--line` border.
- **Center:** stat groups — POSITION (lavender), TIME, BPM (larger), SIG, KEY, LOOP. Each is uppercase `--text-3` label with mono value, separated by 1px `--line-2` rules.
- **Right:** stereo level meter (2 stacked 90×4 bars, gradient green→amber→red), CPU readout in mono.

---

## Arrange View

### Layout
Two-column body inside the view region: `260px` track-list column + flexible timeline.

### Track list (left column)
- Header row: “Tracks” label + small dashed `+` button.
- Each track row: 96px tall (varies with density), `--row-h` CSS var.
  - 28×28 instrument glyph in `--bg-2` rounded square
  - Track name (13/500/`--text-1`) over kind line (10.5/`--text-3`) — e.g. "Drums" / "Kit · Resonance Drums"
  - Right side: 19×19 mono buttons — `M S ● 🎧` (mute / solo / arm / monitor)
- Selected track gets `--bg-2` background and a 2px `--accent` left border.

### Timeline (right column)
- **Ruler:** 28px tall, sticky. Bar numbers in mono, ticks below. Borders only between bars.
- **Section band:** 22px tall just under ruler. Section pill (e.g. "Intro · 6/8 · 90 BPM") spans its bars; lavender wash `--accent-dim` with `--accent-line` border.
- **Lanes:** one per track, same height as headers. Faint vertical bar lines.
- **Clips:** rounded 8px tiles, 8px inset top/bottom.
  - MIDI clips: lavender wash + lavender border, mini note preview (small lavender rects scattered by density).
  - Audio clips: amber wash + amber border, simple bar-chart waveform placeholder.
  - Each clip header has clip name (10.5px medium, accent color) and bar count in mono.
- **Playhead:** 1px warm vertical line with rounded warm "tab" at top.

### Toolbar (above body)
Section pills (Intro / Verse) with colored dot + label + range, plus dashed `+ Section`. Right side: `Edit section`, `Export chords`, `View` ghost buttons.

---

## Mixer View

Two-column body: scrollable strip lane (left) + 320px Inspector (right).

### Toolbar
"MIX" label, segmented control with `Tracks / Buses / FX`. Right side: `Snapshot`, `Link`, `Layout`.

### Channel Strip (132px wide, ~600px tall)
Anatomy top→bottom inside `--bg-2` card with `--line-2` border, radius 12:
1. **Head:** 22×22 lavender glyph + track name. Bottom-bordered.
2. **M S ● 🎧 grid:** 4 mini buttons (20px tall) in `--bg-1` with mono labels.
3. **Instrument slot:** lavender-tinted pill containing a `◆` glyph and instrument name (e.g. "Resonance Wave"). Empty slots read `+ Instrument`.
4. **FX inserts list:** stacked `--bg-1` pills (10.5px) with the plugin name. Empty terminal `+ FX` is a dashed 1px placeholder.
5. **Knob row:** 28×28 PAN and SEND knobs. Knob is a circle with a 1.5px lavender indicator that rotates -130°…+130° based on value.
6. **Fader area:** vertical fader track (22px wide, `--bg-1` with `--line-2` border, tick marks at 0/.25/.5/.75/1) + 14px lavender fader cap with mid-line indicator. Beside it, two 4px stereo meter columns.
7. **dB readout:** value in mono + small "dB" label, top-bordered.
8. **Routing footer:** "OUT" label + `→ Master` value.

Selected strip gets `--accent-line` border and a 3px `--accent-dim` outer ring.

### Bus & Master strips
- **Bus:** warm tint (amber). Slightly different head color, no instrument slot. Same fader anatomy with warm fader cap.
- **Master:** wider (156px), gradient lavender wash, "MASTER" centered uppercase header, two big numeric readouts (OUT dB / PEAK), tall meters, tiny "BOUNCE" tag at the foot.
- A vertical dashed divider separates tracks from busses, with a vertical "BUSES" label.

### Inspector (right column)
Three groups, each with uppercase letterspaced title + bottom hairline:
- **SIGNAL:** 2×2 stat tiles (Peak / RMS / Pan / Out) — `--bg-1`, mono values
- **ROUTING:** dashed-bottom rows — Input / Output / Send A / Send B
- **CHAIN:** plugin rows with lavender bullet, plus a dashed "Add to chain" placeholder

---

## Compose View

Two-column body: workspace (left, scrollable) + 280px right rail.

### Toolbar
Section tabs (Intro / Verse / Chorus) with colored dot + uppercase label + mono range, plus dashed `+ Section`. Right side: `Edit section`, `Export chords`.

### Workspace, top to bottom
1. **Scale stripe** (`--bg-2`, radius 12, padded 10/16): "SCALE" label, italic-serif "B minor", "natural · 7 notes", and 7 note pills on the right (B/C♯/D/E/F♯/G/A). Active note (B) gets the lavender tint.
2. **Chord lane:** 150px side label ("Chords" / "Post-Rock · 5 chords") + 8-column chord card grid.
   - **Chord card:** 64px min-height, `--bg-2`, radius 9. Top row: roman-numeral degree (mono uppercase, `--text-3`) + tiny lock dot if locked. Big italic-serif chord symbol (22px). Bar count footer.
   - Playing chord: lavender wash `--accent-dim` + lavender border.
   - Empty slot: dashed border with centered `+`.
3. **Three instrument lanes** (Synth Bass / Synth Pad / Lead Synth): 64px piano-roll mini view per row. Lane title + meta on the left (150px). The mini view is a 5-row grid with notes drawn as 1×1 lavender rects at varying x offsets — read as low/mid/high register based on the source variant.
4. **Drum lane:** label ("Drums" / "16 steps · 6/8") + 16-step grid panel.
   - Header row: bar markers (1, 2, 3, 4) every 4 cells.
   - 7 instruments (Kick / Snare / Clap / Hat C / Hat O / Tom / Perc).
   - Cells: 18px tall, radius 3. Off = `--bg-1`. On = filled with the row's color (kick=lavender, snare/clap=amber, hats=neutral, toms/perc=lavender-soft).

### Right rail
Two grouped sections, each with title + bottom hairline:
- **Chord generator:**
  - `Style` select (default "Post-Rock")
  - `Chords` and `Beats / chord` steppers (5 / 4) side by side
  - `Start °` / `End °` selects ("(any)" / "(any)")
  - `Seventh chords` toggle
  - **Generate** primary lavender button + `↻` regenerate ghost button
  - Mono seed footer
- **Section motif:**
  - `Source` select ("Manual")
  - `Complexity` slider (0.35) — 3px track, lavender fill, white-over-lavender thumb
  - Motif preview card: "MOTIF" label + "9 notes" mono, 60px area with lavender note dashes scattered by step
  - Help copy in `--text-3`

---

## Interactions & States

- **View tabs:** click switches view in-place. No transition.
- **Track selection (Arrange):** click on track header selects that track; selection highlights both header and lane.
- **Strip selection (Mixer):** click on a strip selects it and updates the right Inspector.
- **Section tabs (Compose):** click switches the active section.
- **Chord cards:** clicking should toggle lock (locked = solid border + small dot, currently rendered for chord 0). The "playing" state is rendered for chord 4.
- **Hover:** any clickable surface should add a subtle background (`--bg-3`) without changing layout.
- **Play button:** toggles `playing` state; swap play/pause icon. (Production should bind to the engine's transport.)
- **Density tweak:** changes `--row-h` (72 / 96 / 116) — Arrange row height + flow on. Equivalent in the production app would be 3 user-selectable density modes.
- **Accent tweak:** updates `--accent` and derived variables across the entire UI at runtime.

## State (rendered, not yet wired)
- `view: "Arrange" | "Mixer" | "Compose"`
- `playing: boolean`
- `selectedTrack: number` (Arrange + Mixer)
- `activeSection: number` (Compose, Arrange section pills)
- `tweaks: { accent, density, view }` — persistence to disk in production: respect existing user-prefs storage.

## Iconography
Custom 24×24 stroke icons (1.5 stroke) — see the `I` map in `app.jsx` for play / pause / stop / rec / prev / next / metronome / loop / mute / solo / arm / headphones / freeze / link / trash / search / chevron / plus / more / sliders / waveform / midi. Replace with whatever the codebase already uses; ensure stroke weight and corners match the existing icon set, otherwise import these.

## Assets
No bitmap assets. All glyphs and previews are SVG inside the components. Fonts pulled from Google Fonts at the top of `Resonance.html` — production should self-host:
- Geist (300–700)
- Geist Mono (400–600)
- Instrument Serif (regular + italic)

## Files in this bundle
- `design/Resonance.html` — root file; load in a browser to see the prototype
- `design/app.jsx` — chrome + transport
- `design/arrange.jsx` — Arrange view
- `design/mixer.jsx` — Mixer view
- `design/compose.jsx` — Compose view
- `design/main.jsx` — app shell + tweaks wiring
- `design/tweaks-panel.jsx` — runtime tweaks panel (drop in production; not needed for shipping)

## Notes for the implementing developer
- The prototype renders inside a fixed 16:10 stage container. In production the layout should fill the OS window — strip the stage wrapper and let the chrome+transport+view stack flex to the available height.
- Strip the React + Babel CDN imports; build with the project's existing toolchain.
- The `useTweaks` hook in `tweaks-panel.jsx` is a designer-time tool. Do not ship it.
- Keep monochrome instrument coding (per design decision). Track-color hues were intentionally rejected in favor of the lavender + amber semantic split (MIDI vs audio/bus).
- Selection should always be a `--accent-line` border or a `--accent-dim` wash — never a heavy fill.
