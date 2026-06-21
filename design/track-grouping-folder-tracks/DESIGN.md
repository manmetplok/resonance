# Design: Track grouping & folder tracks (epic #36)

**Surface:** Arrange view — track-header column + timeline lanes (with a noted Mixer
reflection). **Prototype:** `design/track-grouping-folder-tracks/index.html`
(self-contained; state switcher in the toolbar, deep-linkable via `#flat`, `#grouped`,
`#folded`, `#member`, `#nested`, `#macro`).

## Intent

Folder/group tracks turn a flat lane list into a navigable hierarchy. A **group** is an
*organisational + macro-control* construct: it folds related lanes under one header,
brackets them with a colour identity, and exposes group-level mute / solo / level trim that
cascade to members. It is deliberately distinct from two existing concepts:

- **FX-return / aux buses** (epic #31) — signal routing destinations. A group is not a bus;
  it does not introduce a return channel. (Where the engine's existing bus summing is a
  convenient implementation detail for the macro level, that stays hidden — the user model
  is "a folder of tracks I can control together.")
- **Instrument sub-tracks / vocal stacks** (`SubTrackLink`, epic #23) — fan-out ports of one
  plugin. Members of a group are still full, independent tracks; the prototype labels vocal
  sub-tracks "Vocal · sub-stack" so the two never read as the same thing.

## Key decisions

1. **Group header is a first-class row** (60px, vs 96px track rows), not a modal or sidebar.
   It lives inline in the timeline so structure is visible while arranging. Anatomy, left→right:
   caret (▾/▸) · colour swatch · **bold group name** · `N trk` count badge · group level
   trim (mini-slider + dB) · **M** / **S** macro buttons. The whole header row carries a
   faint group-colour wash so it reads as a band, and its timeline lane shows a "spans all
   members" tint when expanded.
2. **Colour identity via a coloured rail.** Each group gets an identity colour shown as a
   swatch in the header and a 3px rail running down the left edge of every member's header
   cell — a visual bracket tying members to their folder. Members indent; nested groups add a
   second, inset rail so depth reads at a glance. **This needs new tokens.** `theme.rs` has no
   arbitrary-colour palette today (the palette is strictly semantic). The prototype proposes a
   small **GROUP IDENTITY palette** (`--grp-drum`/`--grp-vox`/`--grp-keys`/`--grp-gtr`, each
   with a 14% wash + 34% line variant) — muted jewel tones tuned to the dark lavender
   backdrop, used for *identity only*, never semantics. Architect/dev should add these as
   canonical tokens rather than inlining colours, and pick the swatch from this fixed palette
   on group creation (cycling, user-recolourable).
3. **Collapse = declutter + overview, not just hide.** Folding a group removes its member
   rows from the timeline (and is virtualization-friendly) and repaints the folder's own lane
   as a **consolidated overview**: every member clip flattened into one tinted strip so the
   user still sees "there is content here" at a glance. Fold state is runtime-ish but, unlike
   the global-shelf/inspector carets, it **is persisted** in the project (the epic requires
   fold state to survive save/reload) — note this differs from the "collapse state is never
   persisted" rule for panel chrome; group structure is project content.
4. **Macro cascade is visible and non-destructive.** Group mute/solo flow to members: each
   affected member shows a small **"via group"** chip (pink for mute, amber for solo, matching
   the unified BAD/WARM semantics) and a dimmed lane, while the member's *own* M/S stays
   independent — so ungrouping later restores the right per-track state. Soloing a group dims
   ungrouped tracks too (standard solo behaviour). The group level trim **scales** members'
   contribution (a macro gain), it does not overwrite per-track faders.
5. **Creation from selection.** With multiple tracks selected, a floating action bar
   ("3 tracks selected · … — Group ⌘G") offers grouping; right-click → *Group selected* is the
   menu equivalent. Empty-state (no groups yet) shows a one-card explainer so the affordance is
   discoverable.
6. **Membership by direct manipulation.** Drag a track onto a folder to join it: the group
   highlights as a drop target, an insertion line shows the landing slot, and a chip names the
   destination ("Add to 'Keys'"). Drag a member out to ungroup; drag the header to move the
   whole group (members travel with it). Nesting is supported one level deep.

## Screens & states (all in the prototype)

| State | Covers | What it shows |
|-------|--------|----------------|
| `flat` | **Empty** grouping state | Flat lane list, selected tracks highlighted, explainer card + floating "Group ⌘G" action bar |
| `grouped` | **Populated** — expanded | Drums folder expanded; coloured rail brackets 4 members; header macro controls; "spans all members" lane band |
| `folded` | **Populated** — collapsed | Drums folded to a consolidated overview lane + a second (Vocals) group expanded, so both fold states are visible together |
| `member` | **Interaction** — membership | Drag ghost row, drop-target group highlight, insertion line, destination chip |
| `nested` | **Populated** — hierarchy | A sub-group folder inside a parent folder; stacked rails; per-level macro controls |
| `macro` | **Interaction / edge** — cascade | Group mute (pink "via group" + dimmed lanes) and group solo (amber) cascading; ungrouped track dimmed by active solo |

**Loading / error:** not applicable as discrete screens — grouping is a purely local,
synchronous organisational edit (no async fetch, no network). The nearest "edge" cases are the
macro-cascade conflicts (own-mute vs via-group, solo dimming) which the `macro` state covers.
Project-level missing-asset relinking belongs to import (epic #35), not here.

## On-brand notes for the developer

- Reuse `theme.rs` tokens throughout; the only *new* tokens needed are the **group identity
  palette** above (add to `theme.rs`, don't inline).
- Group-header row height should become a constant (e.g. `GROUP_HEADER_HEIGHT`, 60px) next to
  `TRACK_HEIGHT`; member indent + rail offsets should be constants, not inlined numbers.
- The caret follows the standard `view::controls::collapse_caret` pattern (▾ open / ▸ closed).
- Macro M/S reuse the existing `mute_button` / `solo_button` styling (BAD / WARM); the "via
  group" chip is a new tiny inherited-state badge — combine colour + the text label so it
  never relies on colour alone (per the colour-rules guideline).
- Header column must keep its row-for-row alignment with the timeline canvas and stay
  virtualization-friendly when groups collapse.
- **Mixer reflection (follow-on):** the mixer already clusters parent + sub-track strips and
  tracks `expanded_sub_track_parents`; groups should read there as a coloured cluster with the
  same identity colour. The prototype focuses on Arrange; the mixer treatment can mirror these
  decisions in the architect's breakdown.

## Persistence (for the architect/dev)

Group id, name, identity colour, ordered membership, nesting, **fold state**, and the macro
mute/solo/level all live in the project (replay-diff + undo). Applied identically in live mix
and offline bounce. Out of scope (per epic): full VCA fader automation.
