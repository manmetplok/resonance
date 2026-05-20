#TODO

    - [x] Implement the new drum editor from the design bundle
      (https://api.anthropic.com/v1/design/h/KuEbfCOuhRVlYI14XTJ6sA?open_file=Resonance.html),
      supporting **multiple drum patterns per section** (pattern bank
      / picker + per-section pattern assignment, not just a single
      grid per section as today). Fetch the bundle, read its README,
      and mirror the drum-editor spec in `view/compose/drumroll/**`
      and the underlying drum-pattern state in `state.rs` /
      `update/**`. While here, sweep the rest of the **compose**
      view designs in the same bundle and pick up other improvements
      (lane inspectors, chord lane, melody/bass lanes, vocal lane,
      expanded editor chrome) where they're a clean lift. Route the
      visual work through `ux-design` and have `e2e-tester` rebaseline
      / add snapshot coverage for the drum editor + any other compose
      surfaces that change.
      **What changed:**
      - Promoted the flat project-scoped `drum_groups: Vec<DrumGroup>`
        into a `drum_patterns: Vec<DrumPattern>` bank, with each
        `SectionDefinitionState` carrying `drum_pattern_id: Option<u64>`.
        New `DrumPattern` type lives in
        `resonance-app/src/compose/drumroll/pattern.rs`.
      - `update/compose/drum_groups.rs` rewritten to operate on the
        *managing* pattern's groups. Added `SelectPattern`,
        `AssignPattern`, `AddPattern`, `DuplicatePattern`,
        `DeletePattern`, `RenamePattern`, `SetPatternColor`, and the
        inline-rename trio. `materialize_drum_clips` now resolves
        groups per-section so a single drum track plays different
        rhythms across sections.
      - New `view/compose/drumroll/pattern_picker.rs` renders a chip
        row above the drum lane: chip per pattern, warm-tint border
        on the assigned one, trailing Rename / Duplicate / Delete /
        + Add buttons (uses `ghost_button_style`, theme tokens, no
        ad-hoc colours).
      - Drum Groups Manager modal grew a "PATTERNS" column listing
        the bank; the kit picker scopes pad assignment to the
        active pattern.
      - Persistence: `ProjectFile` carries both `drum_groups`
        (legacy) and `drum_patterns` (current). Loader promotes a
        legacy flat list into a single "Main" entry so older
        projects round-trip cleanly; undo/diff replay mirrors that.
      - Snapshot tests under `resonance-app/tests/
        compose_drum_pattern_picker.rs` lock in four states
        (default, assigned-B-section, renaming, manager modal) with
        goldens under `tests/snapshots/`.
      - **Deferred / future work** (design bundle URL returned
        HTTP 404 from this sandbox, so the visual treatment was
        derived from `ux-guidelines.md` and existing compose-view
        conventions): the other compose-surface sweeps (chord
        inspector polish, vocal lane chrome, melody/bass lane
        treatments, expanded editor chrome) — pick up once the
        bundle becomes reachable, then layer onto the picker /
        pattern bank that's now in place.

    - [x] ! App crashes with segfault on exit (closing the main window).
      Repro: `cargo run -p resonance-app --release`, then close the
      window. Tail of stderr:
      ```
      audio: device="Default Audio Device" sample_rate=48000
        (cpal_default=44100) quantum=256 (probed=Some(256))
        max_quantum=2048 (probed=Some(2048)) buf_frames=2048
        fixed_buffer=true
      WARNING: radv is not a conformant Vulkan implementation, testing use only.
      [1]    2406228 segmentation fault (core dumped)  cargo run -p resonance-app --release
      ```
      **Root cause:** not cpal 0.15 → 0.17 or wgpu drop ordering — a
      pre-existing CLAP host teardown bug that the user finally
      tripped on. Engine-thread `HandlerState` owns
      `bundles: Vec<ClapBundle>` (one per loaded `.clap` shared
      library). On `AudioCommand::ShutDown`, the thread broke out of
      its loop and `state` fell out of scope, which dropped
      `bundles` first → `libloading::Library::drop` →
      `dlclose`-ing every plugin library. But `plugins_arc:
      Arc<RwLock<IndexMap<_, Mutex<SyncClapInstance>>>>` is shared
      with both the main thread (`Resonance.engine.plugins`) and
      with the cpal output-stream callback closure (`_stream`), so
      the live `ClapInstance` values inside *didn't* drop on the
      engine thread — they lived on with function pointers
      (`process`, `destroy`, `deactivate`, `stop_processing`,
      `close_gui`) into now-freed memory. Two crashes followed:
        * the cpal output callback — still running until `_stream`
          is dropped at the main thread's `Resonance` teardown —
          iterates the map and calls `(*plugin).process` against
          unloaded code; the kernel log pinned this on the
          `cpal_alsa_out` thread (PID 2408474, `error 14` =
          instruction fetch from invalid address);
        * later, when `Resonance` finally drops, the plugins Arc
          hits refcount 0 on the main thread and every
          `ClapInstance::drop` (`close_gui` → `stop_processing` →
          `deactivate` → `destroy`) jumps into freed pointers. The
          coredump for PID 2386426 showed exactly this stack:
          `core::ptr::drop_in_place::<ClapInstance>` →
          `Arc<RwLock<IndexMap<…, Mutex<SyncClapInstance>>>>::drop_slow`
          → `core::ptr::drop_in_place::<Resonance>` →
          `iced_winit::run_instance` → `resonance_app::main`.
      The bug had been latent since CLAP support landed; it only
      surfaced now because the latest plugin set + project autoload
      reliably winds up with live instances in the map at exit
      time. cpal 0.17 is a red herring (the lifecycle handshake on
      that side was unchanged — only the `BufferUnderrun` event
      surface moved, fixed in commit `0d2db44`).
      **Fix:** explicit cleanup at the end of `engine_thread` in
      `resonance-audio/src/engine/thread.rs`. After the recv loop
      breaks, swap the plugins IndexMap out under the write lock
      (`let drained = std::mem::take(&mut *plugins_arc.write());`)
      and drop the swapped-out map before `state` (and therefore
      `state.bundles`) falls out of scope. Every `ClapInstance`
      now runs its `Drop` against still-mapped-in libraries; the
      IndexMap is empty when `bundles` unload a few lines later.
      Lock pattern mirrors `handle_remove_plugin`: take write,
      swap out, release lock, then drop the swapped-out container
      so the audio callback's `try_read` isn't blocked longer than
      the swap itself. New regression suite
      `resonance-audio/tests/clap_plugin_drop_order.rs` loads a
      real bundled `.clap` (`target/bundled/resonance-amp.clap`)
      and exercises the exact "drop instance via map clear, then
      drop bundle" pattern with both a single-instance and a
      two-instance setup; skips cleanly when the workspace hasn't
      been built yet. Exposed `ClapBundle` + `SyncClapInstance`
      through the existing `__test_support` module so the test
      can drive the host without making the `clap_host` module
      public. No UI surface touched — pure engine/data-layer
      change, so no `iced_test` rebaseline. (Reported 2026-05-19.)

    - [x] ! Recurring `Audio stream error: Buffer underrun/overrun occurred.`
      messages are spamming the log during normal use — audio output
      glitches even with no plugins / minimal session. Investigate the
      cpal stream callback path in `resonance-audio` (engine thread
      cadence vs. device buffer size, any blocking work in the audio
      callback, sample-rate / buffer-size negotiation after the cpal
      0.15 → 0.17 upgrade). Could be related to the recent cpal bump
      (commit `6ed365b`) — diff the audio thread setup vs. the
      pre-upgrade version. (Reported 2026-05-19.)
      **Root cause:** cosmetic regression from the cpal 0.15 → 0.17
      upgrade, not a new audio glitch. cpal 0.17 added
      `StreamError::BufferUnderrun` and now reports ALSA/JACK xruns
      through the application's `err_fn` callback — previously cpal
      printed them to stderr itself, and ALSA's `PCM.try_recover` is
      now `silent=true` so the engine thread no longer sees those
      lines either. Same audio event (occasional xrun under desktop
      load), now surfaced through our handler which blindly
      `eprintln!`-ed every variant. **Fix:** match on
      `cpal::StreamError::BufferUnderrun` in the output `err_fn`
      (`engine::AudioEngine::new`) and the input `err_fn`
      (`platform::build_input_stream_cpal`), route through a new
      `stream_errors::UnderrunRateLimiter` that emits the first
      occurrence immediately and then coalesces into one summary per
      10s (`UNDERRUN_REPORT_INTERVAL`) carrying the count + lifetime
      total. Other `StreamError` variants
      (`DeviceNotAvailable`, `StreamInvalidated`, `BackendSpecific`)
      are rare and load-bearing — still go through `eprintln!`
      unchanged. New `tests/underrun_rate_limiter.rs` (8 tests) pins
      the rate-limit behaviour, singular/plural formatting, lifetime
      counter monotonicity, and the interval constant. Pure
      engine/data-layer change — no UI surface touched, so no
      `iced_test` rebaseline.

    - [x] Implement the new design for the global tracks (tempo, time
      signature, sections, **chord track**, etc.) using the design update
      here: https://api.anthropic.com/v1/design/h/KuEbfCOuhRVlYI14XTJ6sA?open_file=Resonance.html
      Fetch the design bundle, read its README for context + intent, and
      land the relevant aspects in `resonance-app/src/timeline*.rs`,
      `timeline_draw.rs`, and the global-tracks header. Route the UI work
      through the `ux-design` agent (theme constants, spacing, hover/active
      states) and have `e2e-tester` rebaseline `iced_test` snapshots once
      the visuals are final.
      **What changed:** rebuilt the global-tracks shelf to match the
      bundle's `GlobalShelf` design from `arrange.jsx`. The shelf is
      now a two-part strip: an always-visible 32 px header
      (`GLOBAL_SHELF_HEADER_HEIGHT`) with caret toggle + `GLOBAL` tag +
      count badge + one-line summary `{N}/{D} · {BPM} BPM · {root}
      {mode} · {chord-total} chords`, and (when expanded) three
      stacked lanes — **Chords** (56 px, new) flattened from
      `compose.placements`+`compose.definitions` with section tabs
      and chord blocks tinted by quality (Min* → lavender, Dom7 →
      warm, other → neutral); **Tempo** (40 px, kept) automation
      curve; **Signature** (28 px, kept) pill markers with a
      `compound · N eighths` hint for compound meters. New theme
      constants `GLOBAL_SHELF_HEADER_HEIGHT`, `GLOBAL_TRACK_CHORD_HEIGHT`,
      `GLOBAL_TRACK_TEMPO_HEIGHT`, `GLOBAL_TRACK_SIG_HEIGHT`. The
      canvas's `fixed_header_height` and `draw_global_tracks` plus
      the column-side `view::track_header` got rewritten so the
      shelf paints + clicks line up row-for-row across both sides;
      `timeline_input::handle_press` now routes shelf-header clicks
      to `ToggleGlobalTracks` and lane clicks to the per-event
      handler with the new Y offsets. Rebaselined all four existing
      goldens (`track_header_alignment_scroll_*`,
      `timeline_lane_clip_globals_expanded_scrolled`) and added two
      new ones (`global_tracks_shelf_collapsed`,
      `global_tracks_shelf_expanded`) so the shelf chrome itself
      is locked in. Future-work in the bundle's design that wasn't
      landed: gradient `bg-1 → #131419` backdrop on the header
      strip, the right-side `Detect / From MIDI / +` action chips
      (no message handlers exist yet), and SVG-based glyphs for
      each lane label (currently Font-Awesome substitutes
      `music / wave-square / sliders`).

    - [x] ! Global tracks: editing tempo and time-signature events has
      flaky UI — values don't update reliably, drag/click hit-testing
      feels inconsistent, and the on-canvas markers can lag or stale
      after a change. Likely related to the new `GlobalShelf` rewrite
      (new lane Y offsets in `timeline_input::handle_press`, new
      heights `GLOBAL_TRACK_TEMPO_HEIGHT` / `GLOBAL_TRACK_SIG_HEIGHT`).
      Audit the tempo / signature edit path end-to-end: hit-test math
      in `timeline_input`, the engine round-trip, and the
      tempo/signature lane redraw in `timeline_draw::draw_global_tracks`.
      Add `iced_test` coverage for an edit cycle so regressions land
      on goldens next time. (Reported 2026-05-19.)
      **Root cause:** three independent staleness bugs piled on top
      of each other. (1) `TimelineFingerprint` only tracked
      `tempo_points.len()` / `signature_points.len()`, so editing an
      *existing* point's value (drag, pick_list, transport-bar
      commit) mutated the data but never invalidated `canvas::Cache`
      — the curve / pill markers kept showing the old values until
      some unrelated state change (zoom, scroll, clip move) triggered
      a fingerprint mismatch. `selected_global_event` was also
      absent from the fingerprint, so a fresh click on a tempo dot
      updated state but didn't repaint the dot in the accent color.
      (2) `TransportMessage::CycleTimeSignature` mutated
      `signature_events[0]` but never called `rebuild_and_send_tempo`,
      leaving the GUI-side `tempo_map.signature_points` (the array
      the draw routine actually reads) and the bar table stale; the
      shelf kept showing the pre-cycle numerator/denominator.
      **Fix:** added `tempo_events_hash` + `signature_events_hash` +
      `selected_global_event` to `TimelineFingerprint` so any
      in-place edit or selection click forces a redraw, and added
      the missing `rebuild_and_send_tempo` call to
      `CycleTimeSignature`. Hit-test math in
      `timeline_input::handle_global_track_click` audited and found
      correct end-to-end — the perceived "inconsistency" was the
      stale cache hiding successful clicks. New regression suite
      `tests/global_tracks_edit_cycle.rs` covers the edit cycle:
      four data-level tests (cycle signature, update signature via
      pick_list, update tempo via drag, select / deselect) and two
      snapshot tests (`shelf_after_cycle_time_signature`,
      `shelf_after_update_tempo`) lock the post-edit visuals.
      Added `#[doc(hidden)]` `test_*` accessors on `Resonance` so the
      data-level tests can read state through the public API.

    - [x] Fix compile warnings across the workspace. Run `cargo build`
      / `cargo clippy --workspace --all-targets` and clean up the
      accumulated dead-code, unused-import, unused-variable, and
      deprecation warnings so the build comes back to clean.
      **What changed:** workspace went from 44 clippy warnings (2 also
      visible under `cargo build`) to 0 across `cargo build` and
      `cargo clippy --workspace --all-targets`. Breakdown:
      `resonance-app` had 24 `clippy::question_mark` warnings — every
      one was a `canvas::Program::update` returning
      `Option<canvas::Action<…>>` that used `if cursor.position_in(bounds)
      .is_none() { return None; }` / `let Some(x) = … else { return None; };`
      / `match … { Some(c) => c, None => return None };` patterns
      where `?` is semantically identical; rewrote across
      `midi_editor.rs`, `timeline_input.rs`, `view/knob.rs`, and the
      compose-view canvases (`chord_lane`, `drumroll/canvas`,
      `expanded_editor`, `manual_motif_canvas`, `tracks/canvas`,
      `vocal_lane`, `vocal_roll/canvas_program`). No behaviour
      change — `?` desugars to the same early-return-None on the same
      `Option`. `resonance-music-theory` tests
      (`tests/vocal_params_validate.rs`, 7
      `clippy::field_reassign_with_default`) converted to struct-update
      `VocalParams { field: …, ..VocalParams::default() }` syntax; all
      8 tests still pass. `resonance-music-theory/src/voicing.rs`:
      `Option::map_or(true, …)` → `is_none_or(…)`.
      `resonance-metering/src/spectrum/ring.rs`:
      `clippy::needless_range_loop` → `.iter_mut().enumerate().take(n)`
      (preserving the `unsafe { … }` raw-pointer body).
      `resonance-audio` had two `dead_code` methods (`ClapInstance::
      is_gui_open` and `MidiOutputRegistry::has_output`) — both are
      public probes that look load-bearing for future hosting / engine
      thread call-sites, so suppressed locally with
      `#[allow(dead_code)]` + a one-line justification rather than
      deleting. Existing snapshot tests
      (`global_tracks_shelf_*`, `track_header_alignment_*`,
      `timeline_lane_clip_globals_expanded_scrolled`,
      `mixer_inspector_renders_without_busses`, plus the 38 library
      doctests) all still pass.

    - [x] Mixer: when expanding a parent track that has sub-tracks, the
      sub-tracks are interleaved with unrelated tracks in the strip row
      instead of being grouped immediately next to their parent.
      Sub-tracks should render contiguously beside the parent strip, and
      ideally use a distinct strip color / accent so the parent → child
      relationship reads at a glance. While doing this pass, cross-check
      the mixer against the new design bundle
      (https://api.anthropic.com/v1/design/h/KuEbfCOuhRVlYI14XTJ6sA?open_file=Resonance.html)
      and align the strip layout / spacing / colors with what's specified
      there. Route the visual work through `ux-design` and rebaseline
      mixer snapshots via `e2e-tester` after the changes land.
      **What changed:** `view_mixer` no longer walks `sorted_tracks()`
      linearly. The new pass emits each top-level track followed by
      its sub-tracks (sorted by `output_port_index`, the stable
      engine-side fan-out order) wrapped in a tight
      `row![…].spacing(0)` cluster, then the next unrelated track
      follows with the normal 2 px gap. Sub-tracks no longer render
      through the polymorphic `view_channel_strip` `is_sub` branch —
      they have a dedicated `view_sub_channel_strip` renderer that
      uses the new `MIXER_SUB_STRIP_WIDTH` (88 px, narrower than the
      132 px parent), `MIXER_SUB_STRIP_BG` (= `BG_1`, one shade
      darker than the parent strip's `BG_2` so the cluster reads as
      recessed), and a 2 px lavender left-edge rail
      (`MIXER_SUB_STRIP_RAIL` → `MIXER_SUB_STRIP_RAIL_SELECTED` when
      selected) to telegraph the parent → child relationship at a
      glance. The sub-strip's control set drops record-arm /
      monitor / FX-list / instrument-pill since none of those apply
      to a plugin-fed sub-track — keeps M / S / FX-bypass, pan, and
      the fader (`MIXER_STRIP_HEIGHT` reused so faders line up across
      the cluster). New `seed_demo_with_drum_subtracks` fixture in
      `demo.rs` builds the worst-case shape (drum parent at
      `order=0`, unrelated tracks at `order=1..=3`, sub-tracks at
      `order=4..=7`) so the grouping is proven against the same
      interleave the engine produces in practice. Three new tests in
      `tests/mixer_sub_track_grouping.rs` lock in the visuals
      (`mixer_sub_tracks_expanded` / `mixer_sub_tracks_collapsed`
      snapshots) and the structural ordering guarantee
      (`mixer_sub_track_render_order_groups_with_parent` —
      Drums(1), Kick(10), Snare(11), HH(12), Tom(13), Bass(2),
      Pad(3), Lead(4)). Added `test_registry`,
      `test_expanded_sub_track_parents`, and
      `test_collapse_sub_track_parent` accessors on `Resonance` so
      the integration test can drive state through the public API.
      The design-bundle URL was not reachable in this environment
      (gated service), so the visual treatment was derived from the
      existing `ux-guidelines.md` palette + established mixer strip
      conventions; future work if a later task reaches the bundle: a
      parent-strip header chip with the sub-track count, and a
      consistent gradient backing across the cluster.

    - [x] ! Track header still bleeds through the transport bar when
      scrolling vertically in the arrange view. The earlier fix wrapped
      the `TimelineCanvas` lane paints in `frame.with_clip(...)`, but the
      **track-header column** (left-side track strips) is a separate
      widget tree and still paints over / through the transport bar at
      the top. Likely the track-header scrollable lacks an opaque
      background above its content rect, or its parent column doesn't
      clip to the area below the transport bar. (Reported 2026-05-19.)
      **Root cause:** `view::track_header::build_track_headers`
      composed the column as `column![ruler, section_band?, global_shelf,
      lane_labels?, lane_area]` where `lane_area` used negative top
      padding (`-frac_offset`) to drive sub-row fractional scroll. In
      iced 0.14, `container.clip(true)` only narrows the `viewport`
      passed to children's `draw()` — it does **not** clip child
      `fill_quad` background paints, so a negatively-padded
      `view_track_header` rendered its background and selection stripe
      over the global-shelf strip / section band / ruler above the
      lane area. The user described it as "bleeding through the
      transport bar"; in practice the column's outer `clip(true)`
      protected the transport bar one level higher but the bleed was
      plainly visible *inside* the header column above the lane area
      (e.g. with the shelf expanded + `scroll_y = 100` you'd see Synth
      Bass's glyph + name printed inside the Signature row, and with
      the shelf collapsed + `scroll_y = 50` the drums track's mute /
      monitor / arm / eye buttons printed inside the `GLOBAL` strip).
      **Fix:** restructure `build_track_headers` into a two-layer
      `stack![lane_subtree, chrome_subtree]`. The chrome subtree is a
      column of `[ruler, section_band?, global_shelf, lane_labels?,
      Space::Fill]` with opaque BG_1 / GLOBAL_TRACK_BG backgrounds; the
      lane subtree is `[Space::height(chrome_h), lane_area]` so the
      lane area's Y origin still lines up with the canvas's first lane
      row. Stack layering paints the chrome ON TOP of the lane subtree
      → its opaque backgrounds mask any upward overflow from the
      negative-padded lane content. `Space::new()` is fully click-
      passthrough (no `on_event` / `mouse_interaction`), so events
      below the chrome rows fall through to the lane subtree's
      `view_track_header` `mouse_area` unchanged. Three existing
      goldens (`track_header_alignment_scroll_50`, `_140`,
      `timeline_lane_clip_globals_expanded_scrolled`) all silently
      embedded the bleed and were rebaselined. New snapshot test
      `track_header_no_bleed_into_chrome_expanded` (`scroll_y = 100`,
      shelf expanded) explicitly pins the worst-case scenario.
      Module doc-comment updated to record the layering invariant.

    - [x] ! Adding a track is a bit flaky — possibly clashing IDs? Investigate
      the track-ID allocator (and any places where a new track's ID is derived
      from `len()` or similar) to see if concurrent / rapid adds can collide.
      **Root cause:** `handle_create_sub_track` inserted at a caller-picked
      `sub_id` but never bumped the engine's `next_track_id`. After loading a
      project whose sub-track ids fell above the highest non-sub-track id
      (e.g. `forreal.rproj`: parent instr `1000000013` + sub-tracks
      `1000000014–19`), the engine's counter stopped at `1000000014`. The
      next user `+` allocated `1000000014`, the engine's `tracks.insert`
      silently overwrote the existing sub-track, the app-side `TrackAdded`
      handler hit the "already in registry" guard and returned, *and* it
      left `pending_track_preset = Some(…)` so a later successful add would
      inherit the dropped preset. Fix bumps `next_track_id` in
      `handle_create_sub_track`, clears the pending preset on the silent-
      drop guard, and widens replay's `next_sub_track_id` bump to include
      every saved track id (not just sub-tracks).

    - [x] ! Mixer panic after adding preset drum track. Reproduce: add the
      preset drum track to a project, then navigate to the Mixer tab — app
      panics at `resonance-app/src/view/mixer/inspector.rs:450:28` with
      `index out of bounds: the len is 0 but the index is 0`. Likely an
      unguarded `[0]` access on an empty pads/sends/slots vec for the newly
      added drum track. (Reported 2026-05-19.)
      **Root cause:** not the drum preset specifically — any track added
      to a brand-new project triggered it. `UiViewCaches::default()`
      seeded `output_choices` as an empty `Vec`, and
      `view::mixer::inspector::output_block` fell back to
      `choices[0].clone()` when the track's `output` (Master) wasn't in
      that empty list. `view_caches.rebuild_output` is only called on
      bus add/remove, project replay, or demo seed — a fresh project
      that just added its first track never invoked it, so the empty
      default propagated to the first inspector render. **Fix:** seed
      the default `output_choices` with `output_choices_for(&[])` so
      it's never empty, and replace the unguarded `[0]` with a
      synthesized `OutputChoice` matching the track's actual output
      (covers the empty-cache case *and* a track routed to a bus that's
      not in the cached list, e.g. mid-replay). A new `iced_test`
      regression at `tests/mixer_inspector_empty_project.rs` reproduces
      the exact panic before the fix.

    - [x] Arrange view: when scrolling vertically, tracks at the top of the
      scrollable area are visible through / bleed into the header + transport
      bar (z-order or clipping issue — the timeline scrollable likely paints
      over its parent's bounds, or the header isn't rendered on top of /
      doesn't have an opaque background above the scrollable's content rect).
      **Root cause:** `TimelineCanvas::draw_into` painted track row
      backgrounds, audio clips, MIDI clips, and the loop dim overlays
      *without* clipping them to the lane area. The "skip if fully
      above" guard only drops tracks entirely above `header_height`; a
      row straddling `fixed_header_height()` (any partial-row vertical
      scroll) still drew its full background and clip body, painting
      over the ruler + section-pill band + global-tracks header. The
      recording overlay in `draw_overlay_into` had the same gap. **Fix:**
      wrap the lane-region paints in `frame.with_clip(lane_rect, ...)`
      so they're clipped to `Rectangle { y: header_height, height:
      bounds.height - header_height, ... }`. Ruler labels, loop in/out
      vertical lines + handles, and the playhead tab stay outside the
      clip on purpose — they intentionally cross the ruler so the
      handles read as draggable from above the lanes. Snapshot tests
      rebaselined and a new `timeline_lane_clip_globals_expanded_scrolled`
      golden added to catch the regression when the globals row is expanded.

    - [ ] Wrap the lane_inspector body in `iced::widget::lazy` fingerprinted on
      `(selected_lane, definition.id, version_counter)`. **Blocked**: iced 0.14
      `lazy` still requires `View: Into<Element<'static, ...>>` (re-verified
      after the 0.13 → 0.14 upgrade). The vocal lyrics block uses
      `text_editor(&Content)` whose widget keeps a borrow alive, so the
      element isn't `'static`. Fixing requires moving `Content` behind `Rc<RefCell<>>`
      (behaviour change) or splitting the inspector so the text_editor lives
      outside the lazy boundary.
    - [ ] Compile-time CMU dict via `phf_codegen`. **Deferred**: 135 k entries
      blow up build time (phf perfect-hash search is exponentially slower past
      ~100 k keys). Runtime parse is ~50 ms once via `OnceLock` — not load-bearing.

## Code review 2026-05-19 — High (deferred refactors)

These were flagged High by the review but are bigger refactors than the
inline fix pass tackled. Each is documented enough to pick up later.

### resonance-app — view-layer perf / architecture

- [ ] `view/track_header.rs:174-187` — manual track-list virtualization has
  the top-skip pad but no bottom-side break; tracks below the viewport still
  allocate widget trees inside `view_track_header`. With 50+ tracks the cost
  shows; with 200+ it dominates.
- [x] `view/track_header.rs:442`, `view/compose/lane_inspector/chord/body.rs:
  44,55,85`, `lane_inspector/instrument/{pad,melody,bass}.rs:21-22` — static
  `Vec<u8>` option lists (`(1..=16).collect()`, `(36..=84).collect()`, etc.)
  rebuilt every frame. Use `OnceLock<&'static [T]>` or the `display_pick!`
  macro pattern.
  What changed: added six `OnceLock<Vec<…>>`-backed accessor functions
  returning `&'static [T]` slices that `pick_list` borrows directly —
  `numerator_options` / `denominator_options` in `view/track_header.rs`,
  `count_options` / `beats_options` in `chord/body.rs`, and
  `register_low_options` / `register_high_options` / `bass_base_note_options`
  in `instrument/mod.rs` (the three vecs are pre-mapped to `NotePick`, so
  pad/melody/bass no longer do `iter().map(NotePick).collect()` either).
  Matches the existing pattern in `vocal/common.rs`. Pure perf — option
  contents and dropdown order are byte-identical, no pixel shift. Workspace
  `cargo build` + `cargo clippy --workspace --all-targets` stay at zero
  warnings.
- [x] `main.rs:223-403` — god-object methods on `Resonance` (tempo/signature
  mutators, plugin-index trio, `track_id_at_arrange_y`). ARCHITECTURE.md
  flags this. Split into `state/plugin_index.rs`, move tempo mutators into
  `update/global_track.rs`.

  What changed: moved the four `impl Resonance` clusters that were piled
  onto `lib.rs` into the modules that already own their concern.
  Plugin-index trio (`with_plugin_mut`, `with_plugin_mut_linear`,
  `insert_plugin_index`, `remove_plugin_index`, `rebuild_plugin_index`)
  now lives in `state/plugin_index.rs`. Tempo / signature helpers
  (`rebuild_and_send_tempo`, `rebuild_tempo_map`, `sync_tempo_display`,
  `remove_tempo_event`, `remove_signature_event`) joined the
  `GlobalTrackMessage` handler in `update/global_track.rs`.
  `track_id_at_arrange_y` moved to a new `state/arrange.rs` — it reads
  across `registry.tracks` plus the arrange-view scroll offset, so it
  belongs next to the rest of the arrange geometry. All callers stay on
  the same `Resonance::method(...)` form (inherent impls split across
  files); no signatures, no behaviour changes. `Resonance::test_*`
  accessors untouched, `tests/global_tracks_edit_cycle.rs` and every
  other workspace test still pass; `cargo clippy --workspace
  --all-targets` stays at 0 warnings.
- [ ] `view/compose/expanded_editor.rs:717`, `chord_lane.rs:611`,
  `vocal_lane.rs:561`, `drumroll/canvas.rs:501`, `tracks/draw.rs:472`,
  `vocal_roll/notes.rs:442` — view files past the ~500-line ceiling. Split
  per the `timeline.rs` / `timeline_draw.rs` / `timeline_input.rs` model.
- [ ] `update/project_io/replay_diff.rs:743` — inline tests outside the
  ARCHITECTURE.md exception list. Either add this file to the exception
  list or move tests to `tests/<feature>.rs`.
- [ ] `state.rs` (735 lines, 17 types) — split into a `state/` directory by
  domain.
- [ ] `update/track.rs:297` — `pending_track_preset = Some(*preset.clone())`
  double-clones a `Box<TrackPreset>`. Use `(*preset).clone()` or store the
  `Box` directly.

### resonance-audio — engine

- [ ] `mixer/master.rs:115-120` + `engine/thread.rs:681-682` — master/track
  peak metering races (`fetch_max(Relaxed)` vs `swap(Relaxed)`). Use `AcqRel`.
- [ ] `mixer/click.rs:50-71` — `render_count_in_clicks` divides by `bpm`
  loaded from tempo array without clamping. Project load could yield 0 → Inf.
- [ ] `engine/bounce/render.rs:78-82,169,236,278,356,401,428` — bounce thread
  takes blocking plugin locks per chunk, glitching live playback. Use
  `try_lock` + retry, or freeze a snapshot.
- [ ] `types/track.rs:57` — `plugin_ids: Vec<PluginInstanceId>` mutated under
  tracks write lock while audio reads. ArcSwap the chain.

## Code review 2026-05-19 — Medium

### resonance-audio
- [ ] `engine/bounce/render.rs:78-82,169,236,278,356,401,428` — bounce thread takes
  blocking `mutex.lock()` per plugin per chunk, glitching live playback. Use
  `try_lock` + retry, or freeze a snapshot.
- [ ] `mixer/master.rs:115-120` + `engine/thread.rs:681-682` — master/track peak
  metering races (`fetch_max(Relaxed)` vs `swap(Relaxed)`). Use `AcqRel`.
- [ ] `mixer/click.rs:50-71` — `render_count_in_clicks` divides by `bpm` loaded from
  tempo array; `handle_set_tempo_events` (`thread.rs:363-368`) doesn't clamp.
- [ ] `engine/mod.rs:435-437` — `AudioEngine::send` silently drops commands.
  Return `Result` or emit an `EngineDisconnected` event.
- [ ] `types/track.rs:57` — `plugin_ids: Vec<PluginInstanceId>` mutated under
  tracks write lock while audio thread reads. ArcSwap the chain.
- [ ] `engine/clips.rs:67,184` — `compute_waveform_peaks` blocks engine thread
  while pushing into `clips_arc.write()`.
- [ ] `engine/midi/clips.rs:127-164` — `MidiClipMoved`/`MidiClipTrimmed` events
  fire even when the clip wasn't found. Move event emission inside the `Some`
  branch (already correct for audio clips).
- [ ] `platform.rs:359-368,495-500` — env-var manipulation racy with anything
  else that reads env mid-stream-build. Cpal-workaround; long-term needs a cpal
  API change.

### resonance-app
- [ ] `update.rs:106-208` — `update()` mixes undo bookkeeping + gate logic +
  dispatch. Extract `record_undo` / `gates_message` helpers in `undo.rs` so
  `update()` reads as a 10-line orchestrator.
- [ ] `state.rs:676-690` — `sorted_tracks()` / `sorted_busses()` allocate a Vec
  per call despite the invariant. Return `&[T]`.
- [ ] `update/project_io/replay.rs:387-409` — plugin slot reconstruction races
  `PluginAdded` events. Defensive sort by saved order after all replays complete.
- [ ] `update/clips.rs:316-318` — `samples_per_tick` ignores tempo-map variation
  when trimming MIDI clips. Use `tempo_map.tick_to_abs_sample`.
- [ ] `update/project_io/replay_diff.rs:743` — inline tests outside the
  ARCHITECTURE.md exception list. Either move under `tests/` or add to the doc.
- [ ] `state.rs` (735 lines, 17 types) — split into a `state/` directory by
  domain (`tracks.rs`, `interaction.rs`, `viewport.rs`, ...).
- [ ] `timeline_draw.rs:219-244` — `draw_global_tracks` rasterises a trapezoidal
  fill as hundreds of `fill_rectangle` calls. Use a single `canvas::Path`.
- [ ] `view/mixer/mod.rs:23-107` — `view_mixer` builds two scrollable rows of
  strips with no row-level virtualization. Wrap in `lazy` keyed on track ids.
- [ ] `view/compose/page.rs:104-113` — `track_count` computed via
  `iter().filter().count()` every frame. Cache on `ComposeState`.
- [ ] `main.rs:169-194` — `parse_startup_tab()` / `parse_demo_flag()` iterate
  `std::env::args()` twice. Parse once.
- [ ] `engine_events/mod.rs:21` — `impl Resonance` god-object. ARCHITECTURE.md
  marks it as a historical exception; convert to a free
  `pub fn handle_engine_event(r, ev)` in `engine_events/dispatch.rs`.
- [ ] `recent.rs:54-57` — re-runs `e.path.exists()` for every entry on load.
  Slow on NFS/removable media. Defer to user click on "Open".

### resonance-music-theory
- [ ] `generator/markov.rs:186` — `history.remove(0)` inside fill loop is O(n²).
  Use `VecDeque` or sliding-window index.
- [ ] `generator/markov.rs:244` — order-2 back-off iterates `&table.transitions`
  per slot. Cache per `suffix`.
- [ ] `derive/motif_engine/build.rs:44-46` — pattern-index `ceil` should be
  `floor` so low `complexity` actually produces simple patterns.
- [ ] `derive/motif_engine/phrase.rs:289-296` — consequent resolves to "lowest
  chord tone" not "root". Use `nearest_midi_to(chord.root, last.note)`.
- [ ] `derive/motif_engine/harmony.rs:71` — `apply_gap_fill` uses `Vec::insert`
  in a loop. O(n²) but ~16 notes per phrase; comment the assumption.
- [ ] `derive/motif_engine/build.rs:130-131` — `snap_to_chord_interval` does
  `i8` subtraction that could overflow if `chord_intervals` ever returned a
  value outside 0..12. Use `i16` defensively.

### resonance-metering / resonance-dsp / resonance-common
- [ ] `resonance-metering/src/spectrum/ring.rs:78-88` — `push_slice` issues N
  atomic RMW pairs per sample. Add a "reserve + bulk write + commit one Release"
  variant.
- [ ] `resonance-metering/src/spectrum/fft_worker.rs:65-70` — fixed-size buffers
  stored as `Vec<f32>`. Use `Box<[f32; FFT_SIZE]>` for unambiguous BCE.
- [ ] `resonance-metering/src/spectrum/fft_worker.rs:81` — worker uses
  `park_timeout(16ms)`; push side never `unpark()`s. Either unpark on
  HOP_SIZE crossings or document the latency.
- [ ] `resonance-metering/src/true_peak/polyphase.rs:64-74` — 48 modulos per
  input sample. Use linear history.
- [ ] `resonance-metering/src/lufs/integrated.rs:55-60` — `debug_assert!(false,
  …)` on 60-min cap. Replace with `log::warn!`; long sessions are not bugs.
- [ ] `resonance-metering/src/crest.rs:66` — full linear scan of 100 ms ring on
  every `crest_db()`. Either monotonic-deque or document UI-rate readout only.
- [ ] `resonance-dsp/src/delay.rs:23-26` — `tap()` silently aliases when
  `delay > mask`. Add `debug_assert!(delay <= self.mask)`.
- [ ] `resonance-dsp/src/lfo.rs:73-86` — `LFO::next` accepts non-finite rate
  and poisons output. Validate in `new`/`set_rate`.
- [ ] `resonance-dsp/src/biquad.rs:189-193` — `clamp_params` can panic if
  `sr == 0`. `let nyquist = (sr * 0.5).max(20.0);`.
- [ ] `resonance-dsp/src/dynamics.rs:60-67` — `Ballistics::from_times` divides
  by potential 0 sample rate. Add `sample_rate.max(1.0)`.
- [ ] `resonance-common/src/registry.rs:80-93` — `is_installed` re-reads JSON
  per call. Provide batched API.

### plugins/*
- [ ] `resonance-mastering/src/stages/linear_phase_eq/convolver.rs:73-74,109-114,
  132-137` — uses `VecDeque::push_back`/`pop_front` per sample. Capacities are
  pre-reserved so no reallocation, but the deque indirection adds per-sample
  overhead vs. a flat ring with two indices. Worth profiling.
- [ ] `resonance-mastering/src/stages/multiband/mod.rs:101-117` — silently caps
  `frames` if the host exceeds the construction-time `max_buffer`. Reallocate
  scratch on `initialize` (off the audio thread) or assert.
- [ ] `resonance-drums/src/dsp/sampler.rs:282-288` — per-pad `volume`, `pan`,
  `oh_blend`, `balance` are snapshotted once per block but applied per sample
  without smoothing. Pad volume jumps still click on automation; master volume
  is now block-linear-ramped, but the pad params still aren't. Add per-pad
  `prev_*` state and interpolate across the block (NUM_PADS × 4 floats).
- [ ] `resonance-mastering/src/assistant/mod.rs` (150 lines) — `Assistant` type
  + impl in `mod.rs`. Move to `assistant/state.rs`.
- [ ] `resonance-wavetable` — every DSP file lives at crate root
  (`effects.rs`, `engine.rs`, …). Fold under `dsp/`.
- [ ] `resonance-ir/src/convolver.rs` — should be `dsp.rs` (per plugin layout
  convention). Also restructure to consume slices instead of per-sample dispatch
  from `lib.rs:221-271`.
- [ ] `resonance-eq/src/band.rs:151-177` — 24/48 dB cuts use Q=0.707 cascades,
  sagging at cutoff vs. true Butterworth. Use per-stage Q tables.
- [ ] `resonance-eq/src/dsp.rs:84-87` — `db_to_linear(output_gain.next())` per
  sample. Smooth in linear-gain space.

### resonance-plugin / wayland-plugin-gui / resonance-svs
- [ ] `resonance-plugin/src/clap_bridge/process.rs:212-217` — manual
  `MaybeUninit` slice reinterpretation; also silently truncates >8 output ports.
  Use `MaybeUninit::slice_assume_init_mut` and `debug_assert!`.
- [ ] `resonance-plugin/src/clap_bridge/process.rs:71-74` — automation
  param-change events bypass any smoother. Document that plugins must drive
  `smoother.set_target` from `set_plain`.
- [ ] `resonance-plugin/src/clap_bridge/state.rs:80-86` — state load races
  in-flight process-block param events. Verify against CLAP threading rules.
- [ ] `resonance-plugin/src/clap_bridge/ports.rs:67-72` — port-name copy
  includes trailing NUL. Check `AudioPortInfoWriter::set` semantics.
- [ ] `wayland-plugin-gui/src/editor.rs:92-100` — `set_size` never updates
  `self.size`. Subsequent `get_size` returns stale data.
- [ ] `wayland-plugin-gui/src/window_thread/paint.rs:80-93` — EGL sized as
  integer `scale`, viewport as float `pixels_per_point`. Either clamp or wire
  `wp-fractional-scale-v1` through.
- [ ] `resonance-svs/src/audio.rs:29` — negative segment offsets silently
  clamped to 0. Either trim leading samples or document.
- [ ] `resonance-svs/src/stages/vocoder.rs:61-63` — `mel.data.clone()` + f0
  collect per segment. Take by value with `mem::take` like the acoustic stage.

## Code review 2026-05-19 — Low

### resonance-audio
- [ ] `types/clip.rs:252-262` — `pre_touch` reads one byte per 4 KiB page; on
  THP-backed mmaps step by the huge-page size.

### resonance-app
- [ ] `main.rs:300,313,350` — `debug_assert!(result.is_some(), …)` after
  `with_track_mut`. If truly an invariant, use `.expect()` so release fails too.
- [ ] `view/track_header.rs:174-187` — bottom-side break missing in the
  manually-virtualised track list; non-visible tracks still allocate widget
  trees.

### resonance-music-theory
- [ ] `fretboard.rs:81` — search caps at `start..=7u8`; voicings above fret 11
  unreachable for barre-chord variations in the upper register.
- [ ] `fretboard.rs:128-129` — `start_fret <= 1 → 0` collapses fret-1 voicings
  to "open" display. Document or distinguish.
- [ ] `derive/vocal/lyrics.rs:178-184` — locked-line rhyme recovery uses
  exact-text match; editing the locked line silently falls back to slot pattern.
- [ ] `chord.rs:113-119` — `Chord::pitch_classes` allocates per call. Return
  `impl Iterator` or `SmallVec`.
- [ ] `derive/motif_bass.rs:283-296` — `chord_tones_in_register` is
  O(register-span × |pcs|). Use a `[bool; 12]` PC bitmap.
- [ ] `rng.rs:36-41` — `next_range` has modulo bias; undetectable in practice
  for n ≤ 256 but worth rejection-sampling if "uniform" is ever claimed.
- [ ] `generator/markov.rs:11-12` — crate uses both `rand::SmallRng` and the
  custom `XorShift`. Consolidate to one determinism contract.
- [ ] `lib.rs` re-exports — `VocalSinger`, `VocalVoicebank`, `g2p.rs` etc. are
  SVS configuration, not music theory. Consider splitting into a
  `resonance-vocal` crate.

### resonance-metering
- [ ] `correlation.rs:51-73,72-82` — `samples_pushed` unused; first 99 ms of
  output is biased toward 0 from zero-history. Gate readout until full window.
- [ ] `crest.rs:55` — same unused-`samples_pushed` field.
- [ ] `lufs/mod.rs:126-128` — `_LOUDNESS_OFFSET_RE_EXPORT` dead workaround.
- [ ] `k_weighting.rs:84,105` — `assign_prefilter` / `assign_rlb` reach into
  `Biquad`'s `pub` fields. Add an `assign_raw` constructor.
- [ ] `spectrum/octave.rs:60-63` — band-fallback path uses `f32::max` (NaN
  propagating), other branches use manual `if v > peak`. Unify.

### resonance-common
- [ ] `wav.rs:198,221` — `target_len = (input.len() / ratio) as usize`
  truncates to zero when `input.len() < ratio`. Clamp to 1 when input non-empty.
- [ ] `scan.rs:2` — `scan_directory` swallows `read_dir` errors. Return
  `io::Result` or log via `tracing`.
- [ ] `registry.rs:117` — hand-rolled `today_iso` date math. Pull `chrono` from
  another transitive dep instead.

### resonance-dsp
- [ ] `delay.rs:29-35` — `tap_linear` casts negative input to `usize` (saturates
  to 0). `debug_assert!(delay_frac.is_finite() && delay_frac >= 0.0)`.
- [ ] `filter.rs:18` — `OnePole::set_cutoff` doesn't clamp against
  `sample_rate/2`. Document the near-identity passthrough at Nyquist.
- [ ] `rng.rs:13-18` — xorshift32 with seed=0 relies on `| 1` recovery.
  Assert non-zero in debug.

### plugins/*
- [ ] `resonance-wavetable/src/viz.rs:218-229` — `ScopeCollector::publish`
  builds a fresh 2 KB stack buffer + double-copy per block.
- [ ] `resonance-wavetable/src/render.rs:218-229` — inner sample loop iterates
  voices even with both oscs disabled. Early-skip.
- [ ] `resonance-reverb/src/viz.rs:51-55` — `TailHistory::push` stores
  `write_pos` with `Relaxed`; other plugins use `Release`/`Acquire`. Inconsistent.
- [ ] `resonance-compressor/src/viz.rs:50-54` — same Relaxed-store pattern.
- [ ] `resonance-delay/src/dsp.rs:103-105` — LFO uses `(self.lfo_phase *
  TAU).sin()` per sample. Acceptable; polynomial sine if ever profiled hot.
- [ ] `resonance-delay/src/lib.rs:170-185` — `fb.powf(n_taps)` per block for 8
  taps; replace with `fb * fb` accumulation.

### resonance-plugin / wayland-plugin-gui / resonance-svs
- [ ] `resonance-plugin/src/param.rs:347-378` — `TempParamOwned` only used by
  `clap_bridge::state`. Move under `clap_bridge`.
- [ ] `resonance-plugin/src/clap_bridge/gui.rs:57-61` — `set_scale` silently
  ignores the host's scale hint.
- [ ] `wayland-plugin-gui/src/widgets.rs:218-233` — `draw_arc` allocates a
  `Vec<Pos2>` of 49 points per arc per frame.
- [ ] `wayland-plugin-gui/src/window_thread/event_loop.rs:176-202` — main loop
  uses a fixed 16 ms tick; doesn't pace via `wl_surface.frame()` callbacks.
- [ ] `resonance-svs/src/lib.rs:8` — `write_mono_f32_wav` exported but
  `mix_into_timeline` is crate-private. Either expose both or rename.
- [ ] `resonance-svs/src/ds.rs:227` — note-name regex compiled per call. Use
  `OnceLock<Regex>`.

