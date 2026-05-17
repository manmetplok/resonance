#!/usr/bin/env python3
"""Generate a verse + chorus .ds in B minor for the TIGER voicebank.

Song: "Silent Rain" — original lyrics, B natural minor scale.
Verse melody hovers B3–A4; chorus climbs to D5 and resolves down to B3 tonic.
"""

import json
import sys
from pathlib import Path

# Per-syllable phoneme durations as fractions of the note duration.
# Keys are syllable tokens used in the song script below.
SYL = {
    "walk":    [("w", 0.10), ("aa", 0.80), ("k", 0.10)],
    "with":    [("w", 0.10), ("ih", 0.70), ("dh", 0.20)],
    "me":      [("m", 0.20), ("iy", 0.80)],
    "through": [("th", 0.15), ("r", 0.10), ("uw", 0.75)],
    "si":      [("s", 0.20), ("ay", 0.80)],
    "lent":    [("l", 0.10), ("ah", 0.55), ("n", 0.20), ("t", 0.15)],
    "rain":    [("r", 0.10), ("ey", 0.75), ("n", 0.15)],
    "find":    [("f", 0.10), ("ay", 0.65), ("n", 0.15), ("d", 0.10)],
    "a":       [("ah", 1.00)],
    "road":    [("r", 0.10), ("ow", 0.75), ("d", 0.15)],
    "that":    [("dh", 0.20), ("ae", 0.60), ("t", 0.20)],
    "leads":   [("l", 0.10), ("iy", 0.60), ("d", 0.15), ("z", 0.15)],
    "us":      [("ah", 0.70), ("s", 0.30)],
    "home":    [("hh", 0.15), ("ow", 0.70), ("m", 0.15)],
    "hold":    [("hh", 0.15), ("ow", 0.60), ("l", 0.15), ("d", 0.10)],
    "the":     [("dh", 0.25), ("ah", 0.75)],
    "dreams":  [("d", 0.10), ("r", 0.10), ("iy", 0.55), ("m", 0.15), ("z", 0.10)],
    "night":   [("n", 0.15), ("ay", 0.70), ("t", 0.15)],
    "has":     [("hh", 0.15), ("ae", 0.65), ("z", 0.20)],
    "known":   [("n", 0.15), ("ow", 0.70), ("n", 0.15)],
    "stay":    [("s", 0.15), ("t", 0.10), ("ey", 0.75)],
    "un":      [("ah", 0.70), ("n", 0.30)],
    "til":     [("t", 0.15), ("ih", 0.60), ("l", 0.25)],
    "dawn":    [("d", 0.10), ("ao", 0.75), ("n", 0.15)],
    "storm":   [("s", 0.15), ("t", 0.10), ("ao", 0.50), ("r", 0.15), ("m", 0.10)],
    "and":     [("ae", 0.55), ("n", 0.25), ("d", 0.20)],
    "cold":    [("k", 0.15), ("ow", 0.60), ("l", 0.15), ("d", 0.10)],
    "brave":   [("b", 0.10), ("r", 0.10), ("ey", 0.65), ("v", 0.15)],
    "dark":    [("d", 0.10), ("aa", 0.65), ("r", 0.15), ("k", 0.10)],
    "bold":    [("b", 0.10), ("ow", 0.65), ("l", 0.15), ("d", 0.10)],
    "in":      [("ih", 0.70), ("n", 0.30)],
    "your":    [("y", 0.20), ("ao", 0.55), ("r", 0.25)],
    "eyes":    [("ay", 0.75), ("z", 0.25)],
    "stars":   [("s", 0.15), ("t", 0.10), ("aa", 0.50), ("r", 0.15), ("z", 0.10)],
    "take":    [("t", 0.15), ("ey", 0.70), ("k", 0.15)],
    "flight":  [("f", 0.10), ("l", 0.10), ("ay", 0.65), ("t", 0.15)],
    "lead":    [("l", 0.10), ("iy", 0.75), ("d", 0.15)],
    "on":      [("aa", 0.70), ("n", 0.30)],
    "light":   [("l", 0.10), ("ay", 0.75), ("t", 0.15)],
}

# MIDI → Hz for the notes used in the song.
HZ = {
    "B3":  246.94,
    "C#4": 277.18,
    "D4":  293.66,
    "E4":  329.63,
    "F#4": 369.99,
    "G4":  392.00,
    "A4":  440.00,
    "B4":  493.88,
    "D5":  587.33,
}

# Song script: list of (syllable, midi_note_name, duration_seconds).
# AP markers indicate breath pauses (no syllable sung, rest note).
NOTE = 0.5      # base note length
LAST = 0.7      # last note of each line (slightly held)
AP_S = 0.3      # AP rest length

VERSE = [
    # Line 1: "Walk with me through si-lent rain"
    ("walk",   "B3",  NOTE),
    ("with",   "D4",  NOTE),
    ("me",     "F#4", NOTE),
    ("through","F#4", NOTE),
    ("si",     "A4",  NOTE),
    ("lent",   "G4",  NOTE),
    ("rain",   "F#4", LAST),
    "AP",
    # Line 2: "Find a road that leads us home"
    ("find",   "A4",  NOTE),
    ("a",      "G4",  NOTE),
    ("road",   "F#4", NOTE),
    ("that",   "E4",  NOTE),
    ("leads",  "D4",  NOTE),
    ("us",     "F#4", NOTE),
    ("home",   "D4",  LAST),
    "AP",
    # Line 3: "Hold the dreams the night has known"
    ("hold",   "B3",  NOTE),
    ("the",    "D4",  NOTE),
    ("dreams", "F#4", NOTE),
    ("the",    "G4",  NOTE),
    ("night",  "A4",  NOTE),
    ("has",    "B4",  NOTE),
    ("known",  "A4",  LAST),
    "AP",
    # Line 4: "Stay with me un-til the dawn"
    ("stay",   "G4",  NOTE),
    ("with",   "F#4", NOTE),
    ("me",     "E4",  NOTE),
    ("un",     "D4",  NOTE),
    ("til",    "C#4", NOTE),
    ("the",    "D4",  NOTE),
    ("dawn",   "B3",  LAST),
    "AP",
]

CHORUS = [
    # Line 5: "Through the storm and through the cold"
    ("through","F#4", NOTE),
    ("the",    "F#4", NOTE),
    ("storm",  "A4",  NOTE),
    ("and",    "B4",  NOTE),
    ("through","A4",  NOTE),
    ("the",    "B4",  NOTE),
    ("cold",   "D5",  LAST),
    "AP",
    # Line 6: "Brave the dark and brave the bold"
    ("brave",  "E4",  NOTE),
    ("the",    "E4",  NOTE),
    ("dark",   "G4",  NOTE),
    ("and",    "F#4", NOTE),
    ("brave",  "E4",  NOTE),
    ("the",    "D4",  NOTE),
    ("bold",   "B3",  LAST),
    "AP",
    # Line 7: "In your eyes the stars take flight"
    ("in",     "D4",  NOTE),
    ("your",   "F#4", NOTE),
    ("eyes",   "A4",  NOTE),
    ("the",    "B4",  NOTE),
    ("stars",  "A4",  NOTE),
    ("take",   "G4",  NOTE),
    ("flight", "F#4", LAST),
    "AP",
    # Line 8: "Lead me on un-til the light"
    ("lead",   "B4",  NOTE),
    ("me",     "A4",  NOTE),
    ("on",     "G4",  NOTE),
    ("un",     "F#4", NOTE),
    ("til",    "E4",  NOTE),
    ("the",    "D4",  NOTE),
    ("light",  "B3",  LAST),
    "AP",
]

SONG = ["AP"] + VERSE + CHORUS  # leading AP, then verse, then chorus

# Build .ds segment.
ph_seq, ph_dur, ph_num = [], [], []
note_seq, note_dur, note_slur = [], [], []
f0, ts = [], 0.005

for entry in SONG:
    if entry == "AP":
        ph_seq.append("AP")
        ph_dur.append(AP_S)
        ph_num.append(1)
        note_seq.append("rest")
        note_dur.append(AP_S)
        note_slur.append(0)
        f0 += [200.0] * round(AP_S / ts)
        continue
    syl, note, dur = entry
    phs = SYL[syl]
    note_seq.append(note)
    note_dur.append(dur)
    note_slur.append(0)
    ph_num.append(len(phs))
    for ph, frac in phs:
        ph_seq.append(ph)
        ph_dur.append(round(dur * frac, 3))
    f0 += [HZ[note]] * round(dur / ts)

segment = {
    "offset": 0.0,
    "text": "Silent Rain — B minor",
    "ph_seq": " ".join(ph_seq),
    "ph_dur": " ".join(f"{x:g}" for x in ph_dur),
    "ph_num": " ".join(str(n) for n in ph_num),
    "note_seq": " ".join(note_seq),
    "note_dur": " ".join(f"{x:g}" for x in note_dur),
    "note_slur": " ".join(str(s) for s in note_slur),
    "f0_seq": " ".join(f"{x:.2f}" for x in f0),
    "f0_timestep": ts,
}

total = sum(note_dur)
sys.stderr.write(
    f"song duration: {total:.2f}s, {len(ph_seq)} phonemes, {len(note_seq)} notes\n"
)

out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("silent_rain.ds")
out.write_text(json.dumps([segment], indent=2, ensure_ascii=False))
sys.stderr.write(f"wrote {out}\n")
