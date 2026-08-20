# Song forms

A catalogue to choose from, not a menu to default to. Each entry says what the
form is *for* — that is what should drive the pick, not the bar count.

Bar counts assume 4/4 unless stated. All of these are conventions with long
histories of being broken well.

## Pop and rock

| Form | Layout | Bars | For |
|---|---|---|---|
| Verse–chorus | V C V C B C | 8/8 each, ~48 | The default because it works: repetition with a lift. |
| Verse–chorus + pre | V P C V P C B C | pre 4, rest 8, ~60 | When the chorus needs a run-up to land. |
| AABA (32-bar) | A A B A | 8 each, 32 | Tin Pan Alley, jazz standards, early rock. Compact; the B is the whole point. |
| Strophic | A A A A | 8–16 each | Ballads and folk, where the *lyric* develops and the music holds still. |
| Verse–refrain | V r V r V r | refrain 2–4 | The hook is a tag on the verse, not a separate section. |
| Two-part with build | A B A B B' | — | Dance and electronic: the form is a tension curve, not a narrative. |

**Bridge placement.** In verse–chorus the bridge earns its place by being
*different* — new harmony, new register, often the first place a new chord
appears. A bridge that recycles verse chords is a third verse wearing a hat.

## Blues

| Form | Layout | Bars |
|---|---|---|
| 12-bar | I I I I / IV IV I I / V IV I I | 12 |
| 12-bar quick-change | I **IV** I I / IV IV I I / V IV I I | 12 |
| 8-bar | I V IV IV / I V I V | 8 |
| 16-bar | 12-bar with the first line doubled | 16 |
| Minor blues | i i i i / iv iv i i / ♭VI V7 i i | 12 |

`harmony_apply_progression` ships `12-bar-blues` as a preset — pass `key` plus
`preset`. Quick-change and minor blues need explicit `symbols` or `numerals`.

The last two bars are a **turnaround**: they exist to throw you back to bar 1.
Ending a song on a turnaround is a mistake unless it fades.

## Jazz

| Form | Layout | Bars | Notes |
|---|---|---|---|
| AABA standard | A A B A | 32 | B is "the bridge" or "the channel", usually a key move. |
| Rhythm changes | AABA on *I Got Rhythm* | 32 | A: I-vi-ii-V; B: dominant cycle III7-VI7-II7-V7, 2 bars each. |
| ABAC | A B A C | 32 | The C ends the tune instead of turning it around. |
| Modal | one or two vamps | 8–16 | *So What* is 32-bar AABA over two chords; the form is long, the harmony is not. |
| Blues (jazz) | 12-bar with ii-V substitutions | 12 | |

A jazz chart is a **head** plus solo choruses plus the head again. Structurally
that means one definition placed many times — exactly what placements are for.
Write the head once, place it at the top and the bottom, place the changes in
between.

For `harmony_apply_progression`: the advertised `ii-V-I` preset is currently
unreachable (the app lower-cases the name before matching), so write it as
numerals `["ii", "V", "I"]`.

## Classical

| Form | Layout | For |
|---|---|---|
| Binary (AB) | ‖: A :‖‖: B :‖ | Dances, baroque suites. B answers A, often in the dominant. |
| Rounded binary | ‖: A :‖‖: B A' :‖ | Binary that comes home. The ancestor of ternary. |
| Ternary (ABA) | A B A | Minuet and trio; da capo arias. B contrasts in key *and* character. |
| Rondo | A B A C A (D A) | The refrain is the identity; episodes are the variety. |
| Theme and variations | T V1 V2 V3 … | Harmony holds, surface changes. Maps very cleanly onto one definition placed repeatedly with different parts generated per placement. |
| Sonata | Exposition (P–T–S–K) / Development / Recapitulation | Long-form. The development is where material is broken apart, not where new material arrives. |

Classical forms usually want **per-section keys**. Use
`mcp__resonance__section_set_scale` per definition rather than relying on the
global key.

## Progressive

Prog is not a form so much as a refusal to reuse one. Common shapes:

| Shape | Notes |
|---|---|
| Multi-movement suite | Named parts (I, II, III), each self-contained, sharing motifs. |
| Through-composed | No section repeats. Every definition placed exactly once. |
| Riff-and-return | A recurring instrumental figure between contrasting episodes — rondo with distortion. |
| Odd-metre vamp | 5/4, 7/8, 11/8 groupings over a static harmony. |

**Meter changes exist in the DAW but not in this API.** The signature track
supports them; no control method reaches it, and `song_summary` reports the meter
at the playhead rather than the song's. See the skill body — ask the user rather
than inferring. For through-composed material, note that one definition per
section means the chord grid is never shared: that is correct here, and it is why
prog projects have many more definitions than a pop song of the same length.

## Bar math

At tempo *T* BPM in 4/4, one bar is `240 / T` seconds.

| Tempo | Bar | 8 bars | 32 bars |
|---|---|---|---|
| 80 | 3.0 s | 24 s | 96 s |
| 100 | 2.4 s | 19 s | 77 s |
| 120 | 2.0 s | 16 s | 64 s |
| 140 | 1.7 s | 14 s | 55 s |
| 174 | 1.4 s | 11 s | 44 s |

A section starting at bar *S* has its bar *N* at `S + N - 1`. The next section
starts at `S + length`. Both are 1-based; clip- and section-relative beats are
0-based.

**These durations assume one tempo and one meter throughout.** The DAW has a
tempo track as well as a signature track, and `song_summary` reports both
`tempo_bpm` and `time_signature` **as they are at the playhead** — neither is a
statement about the song. On anything with tempo or meter changes, the table
above gives the duration of a passage at that tempo, not the length of the song.
Ask rather than compute.
