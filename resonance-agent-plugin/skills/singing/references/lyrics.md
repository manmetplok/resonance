# Lyrics a voicebank can sing

A voicebank sings exactly what it is given: one syllable per note, every
phoneme in the note's time. Words that read well on a page can still come out
as a smear. The craft below is ordinary songwriting craft, with the places
where a synth is less forgiving than a singer called out, and each mistake
tied to the `song_vocal` field it shows up in.

## Singability

**Speech rhythm first.** Say the line out loud and write down the rhythm you
said it in; only then choose pitches. A melody that fights the natural rhythm
of the words is the main reason a lyric sounds wrong even when every note is
in tune.

**Stress on strong beats and long notes.** Stressed syllables go on the beat
(1 and 3 in 4/4, the dotted beats in 6/8) or on the longer notes; unstressed
syllables on short off-beats and pickups. "to-MOR-row" wants "to" as the
pickup and "MOR" on the beat. Misplaced stress is heard as a wrong word.

**Open vowels on long and high notes.** A held or high note wants an open
vowel ("ah", "oh", "ay"); closed short vowels (the "i" of "sit", the "u" of
"cup") belong on short notes. A long note on a closed vowel sounds pinched,
and a high one strained.

**Consonant clusters need time.** Every phoneme has a minimum length;
`song_vocal` reports a note's `min_duration_ms` next to its `duration_ms`, and
flags `too_short` when it does not fit. "strengths" on a sixteenth cannot be
sung. Give cluster-heavy words at least an eighth at slow tempos and more at
fast ones, or choose a simpler word.

- A syllable's onset consonants are sung **before** its beat, borrowed from
  the previous note's tail, so the vowel lands on the beat. A cluster after a
  very short note squeezes that note.
- At the start of a phrase the lead-in is capped at about a fifth of a
  second, so a long cluster there ("str") is shortened. Start phrases on a
  simple onset where you can.

**One syllable per note — no cramming.** Each syllable starts its own note.
Two syllables on one note is not possible; a multi-syllable word left
unbroken with `syllabify` off is sung as all its phonemes on one note and
heard as a smear.

**Short passing notes glide.** Under the bank's pitch model a very short
passing note (about a quarter of a second) is sung as a singer would, sliding
through it, often well off its written pitch. Put the notes that matter —
the hook, the cadence — on longer values.

**Range per section.** Verses low and narrow (a fifth or sixth), choruses
higher with the hook on the peak note, the whole song within about an octave
and a half. One climax per section; the melody generator already places one.

**Breathing.** A rest of at least an eighth between lines, a beat or more
between phrases. A gap of at least the lane's `rest_threshold_beats` is
silence; a shorter one is sung legato through. Write `AP` where the singer
should audibly breathe: the breath takes the end of the gap, and when the gap
is short it also borrows the end of the previous note.

**Every placement sings the same words.** A lane belongs to a section
definition, so a chorus placed three times sings its lyric three times —
good for a chorus, wrong for verses. Different verse words need different
section definitions.

**The lyric generator is a placeholder.** `vocal_generate` without
`lyrics: false` fills the lane from a small corpus of template lines; the
lane's `theme`, `pov` and `avoid_cliches` are stored but not used. Write the
words yourself for anything that matters, and pass `lyrics: false` from then
on.

**Unusual words get guessed.** Names, invented words and spellings no
dictionary has are pronounced by spelling rules (`g2p` reads `rules`). Read
their `phonemes` in `song_vocal`, and set the right reading with
`vocal_set_pronunciation`.

## The syntax

| Write | Means | Example |
|---|---|---|
| `·` or `-` inside a word | an exact syllable break; each piece is one note | `re·mem·ber` (3 notes) |
| `_` standing alone (alias `+`) | hold the previous syllable onto one more note | `love _ _` (1 syllable, 3 notes) |
| `_` as a piece inside a broken word | hold the piece before it | `a·_·ma·zing` ("a" over 2 notes) |
| `AP` standing alone, uppercase | an audible breath before the next word; takes no note | `the night AP we will sing` |
| `[..]` | raw ARPAbet phonemes, `·` splitting notes | `[l ih · l iy]` (2 notes) |
| a new line (`\n`) | the next lyric line | |

Details that matter:

- **Your breaks are exact.** The pieces are the note labels, and the word's
  phonemes are split at the matching consonant: `func·tion` sings
  "f ah ng k | sh ax n", `fun·ction` moves the "k" to the second note.
- **The piece count picks the pronunciation.** A word with several
  dictionary readings takes the one whose vowel count matches your pieces:
  `ev·ery` (2) and `ev·er·y` (3) are both right, sung differently. Pieces
  that match no reading fall back to an automatic split and set
  `syllable_warning` on those notes.
- **A hold sings the vowel and saves the final consonants for the end:**
  `love _ _` sings "l ah · ah · ah v". A `_` before a line's first word has
  nothing to hold and is dropped.
- **A spaced `-` is a dash, not a hold.** `wait - no` is two syllables.
- **Breaths:** a lowercase "ap" is a word; a trailing `AP` at the very end
  of the lyrics is dropped; `[AP]` is a phoneme block and takes a note of
  its own.
- **`syllabify`** (default on) breaks the words you did not break and leaves
  your breaks and `[..]` blocks alone. Turn it off only to store text exactly
  as typed — then every multi-syllable word needs its breaks.
- **Holds you write vs. holds the generator makes.** A `_` in the lyrics is a
  user hold (`slur` `user`) and survives `vocal_generate`. A melisma from
  `syllable_mode` `mixed` or `melismatic` is a generator hold (`slur` `auto`):
  it is not in the lyrics, not counted as a lyric unit, and the next generate
  replaces it.
- **Lyric edits do not move notes.** After adding a `_` the lane has one
  lyric unit more than notes until you add a note or regenerate the melody —
  `counts_mismatch` says so.

## Pronunciation

Each syllable is resolved, first hit wins: a pronunciation you set, the
project dictionary, the voicebank's own dictionary, the CMU dictionary, then
spelling rules. `song_vocal`'s per-note `g2p` names the layer:
`edited`, `project_dict`, `bank_dict`, `cmu` or `rules`.

`vocal_set_pronunciation {word, phonemes}` sets a word for the whole project,
every lane, on the next render; `vocal_clear_pronunciation` removes it.
Phonemes are lowercase ARPAbet, no stress digits:

```
vowels      aa ae ah ao aw ax ay eh er ey ih iy ow oy uh uw
consonants  b ch d dh f g hh jh k l m n ng p r s sh t th v w y z zh
```

A phoneme the voicebank lacks is sung as its nearest neighbour; one nothing
reaches blocks the render with `unsupported_phonemes`, naming the word.

## Where each mistake shows up

| Symptom in `song_vocal` | Usual cause | Fix |
|---|---|---|
| `counts_mismatch` | the lyric fills more or fewer notes than the lane has | add or remove breaks, holds or notes |
| `too_short` | a cluster or a many-phoneme syllable on a short note | longer note, move the syllable, simpler word |
| `out_of_range` | melody outside the voice's `comfortable_range` | transpose the melody, or another voice |
| `syllable_warning` | your pieces do not match the word's vowels | re-break the word |
| `g2p` `rules` | a word no dictionary knows | check `phonemes`; `vocal_set_pronunciation` |
| a hold you wanted is `auto` | it came from `syllable_mode` | write it into the lyrics as `_` |
