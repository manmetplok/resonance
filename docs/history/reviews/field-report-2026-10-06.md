# Field report 2026-10-06 — Arp xrun, set_send, depth estimate, Room decay

> **Archived 2026-10-07.** Historical record; the current truth is [`features/plugins-instruments/wavetable-synth.md`](../../features/plugins-instruments/wavetable-synth.md), [`features/mixer/aux-sends.md`](../../features/mixer/aux-sends.md), [`features/plugins-effects/reverb.md`](../../features/plugins-effects/reverb.md). Open items moved to [`intents/audio-engine/freeze-aux-send-double-apply.md`](../../intents/audio-engine/freeze-aux-send-double-apply.md), [`intents/control-api/agent-readouts.md`](../../intents/control-api/agent-readouts.md), [`intents/audio-engine/review-leftovers-engine.md`](../../intents/audio-engine/review-leftovers-engine.md).

Status: all four fixed and pinned by tests. §1 `6e8b92d6` and §2 `4f68a98c`
(merged @b1b240aa); §3 `8cc428b5` and §4 `112e7480` (merged @ef475589). Each section below keeps the original diagnosis.

Open follow-up from §3: a freeze render applies aux sends and return
buses render into the freeze cache, so a frozen track's sends may be baked
in **and** applied again on playback (read from the code, not run). Kit-tap
sends now follow the same path. Needs a check.

---

## 1. Over-budget cycle on a wavetable track: page faults on the audio thread

### Symptom

```
audio: dsp load avg 22.3% peak 177.0% | over-budget cycles 1 (lifetime 4) | monitor shortfalls 0 (lifetime 3) | plugin lock misses 0 (lifetime 42) | render 8 threads, critical 4447 µs (track 26), efficiency 47%, join wait 5.9 µs
```

Budget at 48 kHz / q128 is 2.67 ms. Average load 22% is healthy; one cycle
in the 10 s window took ~4.7 ms, and 4.4 ms of it was a single track job
(`critical` is the longest single job in the window,
`resonance-audio/src/cycle_load.rs:120`). Track 26 is **Arp**: a
`com.resonance.wavetable` instance with no effects (8 voices, 2 oscs, 3
unison, Ladder 24, chorus, Analog 55%). That patch normally costs a small
fraction of the budget; one block 30× its usual cost is a stall, not DSP.

### Cause (high confidence)

The wavetable plugin's ~40 MB table bundle is `include_bytes!`-embedded and
aliased in place (`plugins/resonance-wavetable/src/dsp/wavetable.rs:102`,
views built by `load_bundled()` from `Engine::initialize`,
`src/dsp/engine.rs:218`). It is never touched up front, so it is
demand-paged from the `.clap` file: the first note that reads a given
table / frame / mip level page-faults **on the audio thread**.

Measured on the live process (pid 1053053, ~2 min after launch):

- The bundle's read-only mapping of `target/bundled/resonance-wavetable.clap`
  was 42.5 MB with only **12 MB resident** (`/proc/<pid>/smaps`).
- The `SCHED_FIFO` render workers had taken **6 major faults** (1 + 2 + 3)
  and the audio `data-loop.0` thread 1 major + 905 minor
  (`/proc/<pid>/task/<tid>/stat`). A major fault is the RT thread blocked
  on I/O; 7 major faults vs 4 lifetime overruns.
- `target/` is btrfs with `compress=zstd:3`: a major fault is a read plus
  a decompress of the compressed extent — milliseconds. Memory showed 0 GB
  free / 42 GB page cache, so the plugin file's pages are evictable even
  shortly after a build.
- Nothing locks or prefaults file-backed plugin data. `resonance-audio/src/prefault.rs`
  covers the host's own anonymous scratch only; `VmLck` was 216 kB though
  `ulimit -l` is unlimited (README tells users to grant `memlock unlimited`).
  The plugin's code segment was also only 2.5 of 5.5 MB resident — cold
  code paths fault the same way, less often.

Expected pattern: overruns cluster early in a session and fade as the
bundle becomes resident, and the meter names whichever wavetable track
happened to touch cold data first.

Secondary, not this spike: 42 lifetime plugin lock misses, and 143
involuntary context switches on one FIFO render thread (some preemption).

### Proposed fix

In the wavetable plugin, **`mlock` the bundle once per process** from
`initialize()` (main thread, never the audio thread), behind a `OnceLock`
since every instance shares the same static pages. `mlock` both faults the
pages in and keeps them resident; touching alone would let them be evicted
again under page-cache pressure. If `mlock` fails (no `RLIMIT_MEMLOCK`),
fall back to a one-byte-per-4 KB read pass so the session at least starts
warm. `mlock` exists on macOS too.

Not recommended as the primary fix: host-wide `mlockall(MCL_CURRENT |
MCL_FUTURE)`. It pins ~1.3 GB and every future allocation (GUI, undo,
project data), can make allocations fail at the limit, and leaves the
plugin unprotected in other hosts. Possible later opt-in for live /
recording sessions if overruns persist after the plugin fix (it would also
cover cold code and third-party plugins).

### Confirm before fixing

Restart, run `vmtouch -t target/bundled/resonance-wavetable.clap`, play
the same project: the spikes should not recur. Or watch major-fault counts
on the `resonance-rende` threads rise in step with over-budget cycles.

### Test

A wavetable-plugin test that, after `initialize()`, asserts the bundle is
resident (`mincore` over the blob's range, Linux-only) — or, simpler, that
the lock/prefault helper ran (a process-global latch; that needs a
standalone test file per CLAUDE.md's process-global rule).

---

## 2. `track_set_send` with destination + level keeps the old destination

### Symptom

From an agent session: setting `to_bus` and `level_db` in one
`track_set_send` call changed the level but left the send on the old bus.
Changing the destination alone works.

### Cause (confirmed by reading the code)

A stale read-modify-write. `set_send`
(`resonance-app/src/update/control/track/sends.rs:194-229`) dispatches one
`MixerMessage` per changed field inside a compound undo. Each one goes
through `upsert_send` (`resonance-app/src/update/mixer.rs:188-201`), which
copies the send from the `r.aux.sends` mirror, edits **one** field, and
sends a full overwrite `AudioCommand::SetAuxSend`. The mirror only updates
from the engine's `AuxSendChanged` echo, so every message in the batch
starts from the same pre-call snapshot:

1. `SetSendDest` → `SetAuxSend { dest: NEW, level: OLD }`
2. `SetSendLevel` → `SetAuxSend { dest: OLD, level: NEW }`, which wins.

It is wider than reported: any two or more fields in one call collide, and
only the last one dispatched sticks (order: dest, level, pre_fader,
enabled). MCP (`resonance-mcp/src/tools/trackmix.rs:208-213`) and the
params struct (`resonance-control/src/methods/track.rs:494-508`) are fine.

### Proposed fix

Compute the final `AuxSend` once in `set_send` (start from `send`, apply
every given field) and dispatch a single message that sends one
`SetAuxSend` and records one undo step, e.g. a new
`MixerMessage::SetSend(AuxSend)`. The alternative is to make
`upsert_send` patch `r.aux.sends` eagerly so later messages in the batch
see the edit. That is a broader change to the echo-only mirror contract,
so the single-message fix is preferred.

### Test

There is no state assertion on a combined call today:
`resonance-app/tests/control/control_sends.rs:344`
(`set_send_of_four_fields_is_one_atomic_undo_transaction`) checks only
the revision and undo counts, and `resonance-mcp/tests/` has no
`set_send` test. Add to `control_sends.rs`: a send to bus A at 0 dB, then
`track.set_send { to_bus: B, level_db: -9 }`, pump the engine echoes, and
assert `dest == B && level_db == -9`. A second case should set all four
fields and assert all four. Undo of that call must restore all four too.

---

## 3. Depth estimate's return gain wrong for the Room (+12.6 dB vs −8.4 measured)

### Symptom

From an agent session: the depth estimate reported the Room return's gain
as +12.6 dB, while switching the send on and off measured about −8.4 dB.
The Hall was right (−4.8 estimated, about −4.5 measured). Following the
estimate would have left the guitars almost dry.

### Cause (confirmed): sends from drum-kit sub-tracks are never rendered, and the depth measurement counts them anyway

The estimate is **measured, not modelled, and does not depend on the
algorithm.** `fill_depth` (`resonance-audio/src/engine/bounce/measure.rs:266-404`)
renders each return bus with only its feeders' sends (`render_return_stem`,
`stem.rs:609`) and each feeder dry (`render_dry_track_stem`, `stem.rs:672`),
then computes the gain as `10·log10(E_out / Σ E_dry·send²)`. With top-level
feeders it matches send on/off within 0.3 dB for all 9 algorithms (scratch
probe: Room −6.94 vs −6.92, Hall −9.25 vs −9.29).

**The bug is in sub-track sends:**

- The mixer applies aux sends only for top-level tracks. `track_pass.rs:299`
  skips sub-tracks, and line 324 is the only call to
  `apply_track_aux_sends`. **A send from a kit tap (`SM Drums → Snare`
  etc.) is silently never rendered.** That is an audible bug in its own
  right: the snare and toms never reach the Room.
- `add_send` (`resonance-app/src/update/control/track/sends.rs:77-`) does
  not reject a sub-track source, and neither does the GUI.
- `render_return_stem` still counts the tap as a feeder. Its send filter
  is keyed on the top-level parent, which is not a feeder, so the tap's
  **dry** route leaks into the return's output, while its send-scaled
  energy is added to E_in. The tap's dry level gets reported as reverb
  output. The gain is stored per bus, so every track sending to that
  return, the guitars included, inherits the inflated figure.
- Scratch probe: an empty return bus (true gain 0 dB), a guitar sending at
  −10 dB → 0.00 dB. Add a kit tap sending at −12 dB → **+10.83 dB**.

**This is the field-report session's routing.** In the live project,
the Room bus (2000000004, knob 1.1 s) is fed by four guitars and by two
kit sub-tracks: `SM Drums → Snare` at −4 dB and `SM Drums → Toms` at
−5 dB. The Hall bus (2000000000) has only top-level feeders. That
explains why the Room is wrong and the Hall is right.

Secondary caveat, which does not explain the report: Room and Chamber
resonate on sustained pure tones (steady-state wet gain up to +11–12 dB
at extreme settings). On tonal input, wet and dry are coherent, so send
on/off energy differences are unreliable for both Room and Hall. Worth a
line in `reading-meters.md`.

### Proposed fix

1. **Make sub-track sends work.** Apply them when the sub-track's route is
   replayed in `reduce_track_pass`. Sending kit taps to a room is a normal
   move, so rejecting them in `add_send` and the GUI is the worse option.
   Check this against the parallel render path and bit-identity at
   `RESONANCE_RENDER_THREADS=8`.
2. In `render_return_stem` and the `OnlyInto` send filter, treat a
   sub-track feeder as send-only: neither its route nor its siblings'
   routes may reach the output, and its energy counts in E_in only if its
   send is actually rendered.

### Test

In `resonance-audio/tests/mixer/` (it has `multi_out_harness`): an empty
return bus fed by a guitar and a kit tap must report `return_gain_db ≈ 0`
(+10.8 today). A second test: toggling the tap's send must change the
energy on master (it doesn't today).

---

## 4. Room decay "measures short of its knob"

### Symptom

From an agent session: Room with the knob at 0.86 s measured 0.49 s using
the preset's early-reflection lean, and 0.66 s with the balance even. The
skill says it should be within a few percent, so the agent raised the knob
to 1.1 s to land on 0.94 s.

### Cause (confirmed by scratch measurement): a skill overclaim plus a meter artifact, not a DSP bug

**The tank is calibrated correctly.** `room/tank.rs:264-270` passes `decay`
as the mid-band T60 to `DecayBands`; the ER envelope follows it too
(`room/early.rs:205-261`). With the Drum Room params and the knob at
0.86 s, a wet impulse response measures **0.889 s** at 1 kHz T30, and wet
noise stopped measures 0.869 s.

**The skill reads the wrong number.** The skills compare the knob with the
**broadband** `t30_seconds` (`resonance-app/src/update/control/meter.rs:1141`;
per-octave values are in `bands`), measured on the **sending track**
(dry + wet). Two things shorten that number:

- **Frequency-dependent decay is intended.** `high_decay_mult` 0.5 and
  7 kHz damping make the treble decay faster, so broadband reads about
  0.81–0.87× the knob even 100% wet (wet IR broadband: 0.70 s).
- **The dry stop.** On the sending track the dry signal cuts off as a
  cliff before the tail, and the −5 dB start of the T30 fit
  (`resonance-metering/src/decay/program.rs`) lands inside that cliff.
  The lower the tail sits under the dry signal, the shorter the reading,
  which is why the ER/tail balance moves it. `program.rs:50-52` assumes
  "the wet tail dominates"; at ordinary send levels it doesn't.

Program-decay readings at knob 0.86 s, broadband (1 kHz T30 in brackets):

| Wet re dry | Balance −0.35 (preset) | Balance 0 |
|---|---|---|
| 0 dB | 0.68 (0.87) | 0.71 (0.87) |
| −12 dB | 0.52 (0.79) | 0.60 (0.85) |
| −18 dB | 0.33 (0.57) | 0.47 (0.73) |

The agent's 0.49 / 0.66 s fit a wet level of roughly −12 to −15 dB. Its
actual send level is not known. The 1.1 s knob was compensating for the
meter, so the room is now ~25% longer than intended.

**The overclaims:**
- `resonance-agent-plugin/skills/spatial/references/depth.md:172-174`
  says "measures within a few percent of the knob". That holds only for
  the mid-band T30 of a wet impulse response.
- `skills/spatial/SKILL.md:126-128` and
  `skills/mixing/references/reading-meters.md:205` say `t30_seconds` is
  "within about 10 % of the knob even with the dry signal in". That is
  false for Room at ordinary send levels.
- Room's acceptance test (`plugins/resonance-reverb/tests/algorithms/room/acceptance.rs:47-77`,
  ±7%) checks `mid_t30()` of a 100%-wet IR with balance 0. Nothing tests
  the broadband value, dry in the mix, a non-zero balance, or
  `program_decay` on a real reverb (`resonance-metering/tests/decay.rs:302`
  uses synthetic noise only).

### Proposed fix

1. **Skills (required):** verify against the 1 kHz `bands[].t30_seconds`
   (or the 500 Hz / 1 kHz mean), not `t30_seconds`. Drop "even with the
   dry signal in". Say that broadband reads short when `high_decay_mult` <
   1 and when dry dominates the stem. Even 1 kHz drops at −18 dB wet
   (0.57–0.73 s), so the skill should also say to measure with the send
   temporarily raised, or to solo the return. Pick one when fixing.
2. **Meter (optional):** start the fit after the dry cliff rather than at
   a fixed −5 dB, or expose a `mid_t30_seconds` field so skills don't
   have to pick a band.

### Test

A module in `plugins/resonance-reverb/tests/algorithms/room/` that drives
sustained noise through Room (Drum Room params, balance −0.35 and 0), mixes
in dry at 0 / −12 dB wet, runs `program_decay`, and asserts the reading
the skill prescribes is within 10% of the knob. Add a lockstep assertion
that the skill text names that field. Scratch harness for reference:
`$SCRATCH/roomdecay/src/main.rs` (session scratchpad, not kept).
