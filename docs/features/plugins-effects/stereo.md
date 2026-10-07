# Stereo (`com.resonance.stereo`)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/warmth-width-depth.md` §6.2 (slice W7).

A mono-safe stereo width tool. It narrows or widens with M/S, folds the
bass to mono, widens a mono source with one of four wideners (one of which
is labelled as the mono risk it is), and sets balance and rotation. It can
audition the side alone or the mono fold. The editor shows a goniometer and
a correlation history. Every default is transparent, and the default plugin
is a bit-exact passthrough.

## Behaviour

Per sample, in this order (`src/dsp.rs`):

1. **Widen** (`widen_mode`, `widen_amount`), limited to the focus band
   `focus_low` … `focus_high` ("Open" at the top):
   - *Decorrelate* (default widener): a velvet-noise decorrelator makes
     pure side from the mid, `(L + s, R − s)`. The mono sum is unchanged at
     any amount.
   - *Diffuse*: per-side ERB all-pass cascades. Spread is capped at 0.25 of
     an ERB step, so the mono fold's deepest notch stays at −2.3 dB.
     Amount and focus changes crossfade to a new all-pass layout.
   - *Micro-shift*: two detuned voices (±9 cents at 10 and 15 ms) made from
     the mid. Above amount 0.5 the moving mono combs exceed −6 dB, and the
     mode is flagged as a mono risk.
   - *Haas (mono risk)*: the right channel above `focus_low` delayed 1–30 ms
     and lowered 3 dB; the low band stays in time. Always flagged.
2. **Width** (`width`, 0–200 %): M/S side gain.
3. **Mono-maker** (`mono_below`, "Off" at the bottom; `mono_slope` 6 / 12 /
   24 dB/oct): a high-pass on the side only, so the mid and the mono sum
   are untouched.
4. **Balance** (`balance`, shown as C / L n % / R n %), then
   **rotation** (`rotation`, degrees).
5. **Audition:** `solo_side` outputs `(S, −S)`; `mono_check` folds to
   `(M, M)`. Both on is silence, which is correct: the side cancels in mono.

- Width, balance, rotation, Micro-shift level and Haas delay ramp per
  sample. A `widen_mode` change crossfades (`MODE_XFADE_MS`); a switch asked
  for during a fade waits for it to end.
- Every stage is skipped at its neutral value.
- **Latency:** 0 in every mode, by design. Haas and Micro-shift delay only
  the wet part, and that offset is the effect, not something PDC should
  undo.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Plugin | `plugins/resonance-stereo/src/lib.rs` | `ResonanceStereo` (`INPUT_CHANNELS = Some(2)`) |
| Params | `plugins/resonance-stereo/src/params.rs` | `StereoParams`: `width`, `mono_below`, `mono_slope`, `widen_mode`, `widen_amount`, `focus_low`, `focus_high`, `balance`, `rotation`, `solo_side`, `mono_check` |
| DSP | `plugins/resonance-stereo/src/dsp.rs` | `StereoDsp`, `WidenMode` (+ `is_mono_risk`), `MonoSlope`, `Diffuser`, `haas_delay_ms`, `micro_shift_notch_db` |
| Editor state | `plugins/resonance-stereo/src/viz.rs` | `StereoViz`: goniometer ring (`GONIO_POINTS` 512, one point per 4 frames), correlation history (`CORRELATION_LEN` 256), current mode |
| Editor | `plugins/resonance-stereo/src/editor/{mod,scope,widgets}.rs` | `GROUPS` (WIDEN, IMAGE, MONO BASS, AUDITION), goniometer + correlation strip |
| Presets | `plugins/resonance-stereo/src/presets.rs`, `presets/*.json` | 7 factory presets |
| Shared DSP | `resonance-dsp` | `VelvetDecorrelator`, `AllpassDecorrelator`, `DopplerShifter`, `apply_width`, `apply_balance`, `ms_encode`/`ms_decode`, `StereoRotation`, `Biquad`, `DelayLine`; `resonance_metering::CorrelationMeter` |

## Editor

A 46 px header with the preset bar and a mono-risk pill (lit for Haas, and
for Micro-shift above amount 0.5). Body: the goniometer and the correlation
strip (−1 at the bottom, +1 at the top) on the left, the parameter groups in
signal order on the right. `tests/editor_layout.rs` holds the `GROUPS`
table to the declared params: every param is drawn exactly once.

## Factory presets

Init — Transparent, Mono Bass Below 120, Widen Mono Source,
Master — Gentle Width, Vocal — Micro-shift Double, Pad — Diffuse Wide,
Haas — Safe.

## Surfaces

- **GUI / Control API / MCP:** the shared plugin surfaces in
  [README.md](README.md#shared-surfaces).
- **Agent skills:** `spatial/references/width.md` (three keys blocks: the
  mono-maker under wide low-end parts, the widen-mode table with its mono
  risks and presets, and "prefer Decorrelate"). The skill verifies with
  `meter_stems` / `meter_measure` `detail: ["stereo"]` (per-band
  correlation, `side_mid_db`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Bit-exact output, every scenario non-silent and not a passthrough | `tests/dsp_golden.rs` → `tests/golden/dsp_golden.f32` | `./scripts/run-tests.py -p resonance-stereo` |
| W7 behaviour: transparent defaults, Decorrelate mono-sum invariance, mono-maker correlation, width/balance/rotation, auditions, Haas risk flag, zero latency | `tests/stereo.rs` | same |
| Presets: full snapshots, load exactly, render | `tests/presets.rs` | same |
| State round trip | `tests/state.rs` | same |
| Editor layout table ↔ params | `tests/editor_layout.rs` | same |
| Skill keys | `tests/skill_keys.rs` | same |

Not covered: no pixel snapshot of the editor, and the goniometer drawing is
not checked. The warmth build closed with a by-hand look at the editors
still open (see the intent below).

## Decisions and deviations

- No latency is reported in any mode: the delays are the effect, and PDC
  must not undo them. (The `dsp.rs` module comment also gives "a plugin
  cannot report a latency change, gap F2" as a reason. That is stale:
  `HostHandle::set_latency_samples` exists and the IR plugin uses it.)
- Diffuse's spread is capped at 0.25 ERB. Uncapped, 0.5 notched the mono
  fold by −14.6 dB.
- Haas is kept, but labelled "Haas (mono risk)" in its choice label.

## Known gaps

- Editor visuals never checked in a live session:
  [plugin-editor-visual-checks](../../intents/plugins-effects/plugin-editor-visual-checks.md).
- No live agent session has used it on a real project:
  [effects-live-agent-session](../../intents/plugins-effects/effects-live-agent-session.md).
