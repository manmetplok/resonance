# Choosing a voice

`vocal_voicebanks` is the truth about what this machine can sing; the tables
below are what was known when this was written, to choose with before you
call it. Never pick a bank from here without checking it is `installed`.

## Reading `vocal_voicebanks`

Per installed bank:

| Field | What it tells you |
|---|---|
| `id` | what `vocal_set_params` takes as `voicebank` |
| `speakers` | the voice modes, each with its own `comfortable_range`; `effect` marks growl, vocal fry and whisper. Empty for a single-voice bank |
| `default_speaker` | what a bank switch without a `speaker` lands on; never an effect mode |
| `comfortable_range` | `{low, high}` MIDI where a plain line sits clearly — not the voice's extremes. `range_source` `default` means the bank is unknown and the range is a guess |
| `missing_phonemes`, `substitutions` | sounds the bank lacks and what it sings instead |
| `curves` | which of `dynamics`, `tension`, `breathiness`, `pitch_bend` reach the model |
| `models` | whether the bank ships its own duration, pitch and variance models |
| `credit`, `terms_url`, `commercial` | what the user owes the creator |
| `vocoder_installed` | `false`: the bank cannot render until its vocoder is installed |

`available` lists the plok.org banks that are not installed. If
`index_status` reads `fetching`, call again shortly; `unavailable` means no
download list right now (see `index_error`).

## The banks

| Bank | Voice | Comfortable range | Lead speakers | Effect speakers | Terms |
|---|---|---|---|---|---|
| TIGER (`tiger`) | male | C3–A4 | glam, fresh, disco, royal, electric, mystic, vinyl | — | paid; "TIGER by tigermeat" |
| Lilia (`lilia`) | alto | D3–A♯4 | one voice | — | free with credit; "LIEE" — but see below |
| Meiji (`meiji`) | uncertain (sources disagree) | B2–F♯4 | standard, hunter, lilith, phantom | — | permission; LUNAI Project |
| Katyusha | soprano / alto | G3–D5 | standard, strong, delicate, lullaby | — | permission; LUNAI Project |
| Keiro Revenant | tenor | C3–A4 | standard, native, cold, fragile, mellow, aggressive, solid | vocal fry, whisper, growl | permission; LUNAI Project |
| Liam Thorne | baritone / bass | A2–F4 | standard, gentle, tired, deep, powerful, aggressive, rock, crispy | vocal fry, growl | permission; LUNAI Project |
| Allen Crow | male | C3–A4 (open and gentle: G3–D5) | standard, heavy, open, gentle | whisper | permission; LUNAI Project |

A new lane sings TIGER `mystic`. All of these banks ship their own duration
and pitch models, and all but TIGER a variance model and the tension curve.
Pitch bend works on every bank. **No installed bank takes the dynamics or
the breathiness curve** (`curves` reports both `false` everywhere): shape
loudness with note velocities and `velocity_dynamics`, and avoid an airy
sound by the speaker you choose and a tension of 0.5 or more.

Meiji's `phantom` is described as whispery, but it is a lead mode: it sings a
whole line clearly.

## What sounded good

A blind listen of every installed voice on the same phrase (1–5, 2026-10-13):

- **5:** TIGER mystic.
- **4:** TIGER disco, electric, fresh, royal and vinyl; Allen Crow heavy;
  Katyusha standard; Keiro Revenant cold and standard; Liam Thorne deep,
  rock, standard and tired.
- **3:** most other lead modes, Lilia, Meiji hunter, lilith and phantom.
- **1:** every effect mode as a lead voice.

Start from a 4 or 5 in the right range unless the song needs a specific
colour.

In a song (a trip-hop track, 2026-10-13), as backing vocals under a
Katyusha lead:

- **Liam Thorne `rock`** — gritty without being breathy, 100 %
  intelligible, and about 11 dB hotter than Keiro `standard` on the same
  notes. The user's pick for backing.
- **Keiro Revenant `standard`** and **Liam Thorne `standard`** — clean and
  clear (92–100 %), less character.
- **Katyusha delicate** — airy; 86–92 % as a backing voice, and its "light"
  kept coming out as "night".

## Matching the voice to the song

1. **Find the melody's centre** — the pitch it spends most time around —
   and its span. Read it from `song_notes` or `song_vocal`, or decide it
   before writing.
2. **Pick a voice whose `comfortable_range` holds the span**, centre near the
   middle. High lines (up to D5): Katyusha, or Allen Crow's open or gentle.
   Low lines (down to A2): Liam Thorne. The middle: TIGER, Keiro Revenant,
   Allen Crow.
3. **If the song's key will not move, move the melody** by an octave, or
   rewrite the line's extremes. Out-of-range notes are flagged per note in
   `song_vocal` (`out_of_range`).
4. **`voice` is not the singer.** The `voice` setting (`soprano` … `bass`)
   only sets the melody generator's default range. The bank and `speaker`
   are the singer. Set the generator's `range` from the voice's
   `comfortable_range` before `vocal_generate`.
5. **Switching bank resets the speaker** to the bank's default unless the
   same `vocal_set_params` patch sets `speaker`. A single-voice bank refuses
   a speaker.

Two things that are per lane, not per note:

- **An effect colour is a whole lane.** The speaker belongs to the lane, so a
  growled line inside a clean verse needs a second vocal track whose lane in
  that section sings only that line.
- **Two lanes singing the same notes with the same bank, speaker and settings
  render identical audio.** There is no automatic doubling: a double needs a
  different speaker, different notes or different settings
  (`backing-vocals.md`).

`timbre` (`airy`, `warm`, `edged`, `bright`) leans the voice's formant a
little; it does not change the singer.

## Installing a bank

1. Show the user the entry from `available`: name, size, `credit`,
   `terms_url`, `commercial`. Ask before installing — it downloads hundreds
   of MB and **accepts the bank's terms for them**.
2. `vocal_voicebank_install {id}` runs as a job; if it returns still-running,
   block on `job_wait`. It also fetches the vocoder the bank needs.
3. Read the result: `already_installed` (nothing downloaded),
   `vocoder_installed` (`false` means it cannot render yet). A `busy` refusal
   means that bank is already installing; `not_found` means the index does
   not list the id; "upstream file changed" means the index is out of date —
   tell the user, do not retry.
4. `vocal_set_params {voicebank}` on each lane that should sing it, then
   pre-flight and render.

## Terms to pass on

Tell the user, whenever a bank sings in their song, its `credit` line and
what `commercial` means:

| `commercial` | Means |
|---|---|
| `free_with_credit` | free to use; the credit line goes in the release |
| `paid` | commercial use needs a paid licence (see `terms_url`) |
| `permission` | commercial use needs the creator's permission |

Two cautions beyond the field:

- **Lilia's terms forbid AI-generated music and other generative-AI use.**
  An agent writing her part is exactly that. Prefer another bank, and say
  why if the user asks for her.
- The shared vocoders some banks use are licensed for non-commercial use;
  a commercial release needs the vocoder's licence checked as well as the
  bank's.
