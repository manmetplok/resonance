# Width: targets, levers, keys

Reason about width in `side_mid_db` (it moves linearly with width) and treat
`correlation` as the fault detector. With equal L/R energy the two are one
number: side/mid -10 dB is correlation ≈ 0.82, -6 dB ≈ 0.60, 0 dB = 0.

## Targets (from `detail: ["stereo"]`)

| Measure | Healthy |
|---|---|
| Master `correlation` | +0.2 to +0.8, typically 0.3-0.7 |
| Band correlation below 150 Hz | ≥ +0.9 |
| Band correlation 150 Hz-1 kHz | ≥ +0.5 |
| Band correlation above 1 kHz | ≥ 0 |
| `side_mid_db` below 150 Hz | ≤ -20 |
| `side_mid_db` mids | -12 to -4 |
| `side_mid_db` highs | -8 to -2 |
| `side_mid_db` whole mix | about -10 to -4 |
| `mono_loss_db` | mix ≤ 3 dB lost; a track ≤ 6 dB, with no band far worse than its neighbours (a comb) |
| `correlation_windows` | warn when `pct_below_0_3` is over 10; `worst` under -0.1 is a fault |
| `balance_db` | mix within ±1 dB; a section within ±1.5 |
| `haas_lag_ms` | `null` |

`one_sided: true` means a hard-panned mono source: correlation is undefined
there, not "wide". Read `balance_db` for which side.

## 1. Mono the lows

Anything wide that carries energy below 150 Hz (stereo synths, pads, room mics,
a widened part) gets a mono-maker. On a track or bus:

<!-- keys: com.resonance.stereo -->
| Move | Keys and values |
|---|---|
| Start from | `Mono Bass Below 120` |
| Mono-maker | `mono_below` 100-150 Hz (0 is off), `mono_slope` `12 dB/oct` or `24 dB/oct` |
<!-- /keys -->

On the master it is the mastering chain's side high-pass (the `mastering`
skill). Verify: the stem's bands below 150 Hz read correlation ≥ 0.99.

## 2. Pan, then give the level back

`mixer_set_pan` per roles.md. The pan law is **stereo balance: centre is
unity**, and panning attenuates only the far channel. A mono part panned away
from centre therefore loses loudness, and the balance you set in the `mixing`
skill moves with it:

| `pan` (either side) | Loudness lost | Fader back (`mixer_set_volume_db`) | Mono-fold level vs centre |
|---|---|---|---|
| 0.25 | about 1.1 dB | +1 | about -1.2 dB |
| 0.5 | about 2.0 dB | +2 | about -2.5 dB |
| 0.75 | about 2.8 dB | +2.75 | about -4.1 dB |
| 1.0 (hard) | about 3.0 dB | +3 | -6 dB |

Re-measure the track's `lufs_integrated` after the pan and set the fader from
the measured loss, not from this table. A pair panned apart (doubles, stereo
backing) does **not** keep its loudness: each side loses what the table says,
so two doubles at ±1.0 are about 3 dB quieter as a pair than the same two at
centre. Give each track its fader back and check the pair's sum against where
the balance had it.

## 3. Doubles before processing

Two performances panned ±0.8-1.0 are the widest, most mono-safe width there is:
they decorrelate naturally and fold to mono without combs. When a part is
single-tracked and needs width, suggest writing (MIDI) or recording a double
first.

## 4. Decorrelate a mono source (above 150 Hz)

`com.resonance.stereo` on the track. The modes, most mono-safe first:

<!-- keys: com.resonance.stereo -->
| Mode (`widen_mode`) | What it does | Mono risk | Start from |
|---|---|---|---|
| `Decorrelate` | generates pure side from the mid | none: the mono sum is unchanged | `Widen Mono Source` |
| `Diffuse` | all-pass cascade per side | small ripple, slight transient smear | `Pad — Diffuse Wide` |
| `Micro-shift` | small detune plus short delays per side | moving combs, milder than Haas | `Vocal — Micro-shift Double` |
| `Haas (mono risk)` | one side delayed | deep combs in mono; only with a low exclude and a level offset | `Haas — Safe` |

| Key | Use |
|---|---|
| `widen_amount` | how much; 0.3-0.5 is usually plenty |
| `focus_low`, `focus_high` | the band that gets widened; keep `focus_low` at 150 Hz or above so the fundamentals stay dry |
| `width` | plain M/S side gain, 0-2 (1 = unchanged); side gain alone vanishes in mono |
| `mono_below` | still use it under a widened source |
| `balance`, `rotation` | re-centre a lopsided stereo source without re-panning |
| `mono_check`, `solo_side` | audition switches for the user; leave `Off` when measuring |
| Reset | `Init — Transparent` |
<!-- /keys -->

<!-- keys: com.resonance.stereo -->
Prefer `widen_mode` `Decorrelate`: the mono listener hears exactly the
original. Never use Haas on bass, kick or the lead vocal.
<!-- /keys -->

Verify: the stem's `side_mid_db` above 150 Hz up by the amount wanted,
`mono_loss_db` no worse, `haas_lag_ms` still `null` (except by choice with Haas).

## 5. Side shelves and side cuts (M/S EQ)

On a bus or wide track, `com.resonance.eq` with a band's M/S set to Side:

<!-- keys: com.resonance.eq -->
| Move | Keys and values |
|---|---|
| Width and air at the top | `band{n}_kind` `High Shelf` (or `Air`) at 8-12 kHz, +1 to +2 dB, `band{n}_ms` `Side` |
| Clear low-mid mud on the sides | `band{n}_kind` `Bell` at 250-500 Hz, -1 to -3 dB, `band{n}_ms` `Side` |
| Tighten the side lows | `band{n}_kind` `Low Cut` at 100-150 Hz, `band{n}_ms` `Side` |
<!-- /keys -->

A side-only move leaves the mono sum alone.

## 6. Mono fold

The master's `mono_penalty_db` and per-band `mono_loss_db`. A mix loses ≤ 3 dB.
If a band is much worse than its neighbours, find the stem with the same band
problem (`meter_stems` `detail: ["stereo"]`) and fix it there: a mono-maker, a
less risky widen mode, or a smaller widen amount.

## Stop rules

- Low-band correlation under +0.9 after a move: undo it.
- Master `side_mid_db` above about -4 overall, or mono loss past 3 dB: too wide.
- A widener on the master to fix a narrow mix: widen the parts instead.
