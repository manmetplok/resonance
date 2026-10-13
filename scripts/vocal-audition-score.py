#!/usr/bin/env python3
# Re-exec under python3 when started as `sh scripts/vocal-audition-score.py`.
''':'
exec python3 "$0" "$@"
'''
"""Listen to a blind audition one file at a time and score each right after
it plays.

Plays every `<token>.wav` in an audition directory (made by
`scripts/vocal-audition.py`) in a random order, then asks for a score:

    1-5      score it (5 = best) and go on
    r        replay
    n        add a note, then score
    s        skip for now (asked again at the end)
    q        quit; scores so far are kept

Scores go to `scores.json` next to the WAVs after every answer, so a later
run picks up where you stopped (`--rescore` starts over). Which bank and
speaker a file was stays hidden until every file has a score; then the
results are printed ranked, per voice and per bank, from `key.json`.

Usage:
    scripts/vocal-audition-score.py [DIR]       # default target/audition
    scripts/vocal-audition-score.py DIR --rescore
    scripts/vocal-audition-score.py DIR --results   # print results only
"""

import argparse
import json
import random
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
PLAYERS = [
    ["pw-play"],
    ["paplay"],
    ["aplay", "-q"],
    ["ffplay", "-nodisp", "-autoexit", "-loglevel", "quiet"],
    ["mpv", "--no-video", "--really-quiet"],
]


def find_player():
    for cmd in PLAYERS:
        if shutil.which(cmd[0]):
            return cmd
    sys.exit("no audio player found (tried pw-play, paplay, aplay, ffplay, mpv)")


def play(player, wav):
    try:
        subprocess.run([*player, str(wav)], check=False)
    except KeyboardInterrupt:
        print()  # Ctrl+C stops playback early and goes to the prompt


def load_scores(path):
    if path.exists():
        return json.loads(path.read_text())
    return {}


def save_scores(path, scores):
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(scores, indent=2, sort_keys=True))
    tmp.replace(path)


def ask(token, done, total, player, wav):
    note = None
    while True:
        try:
            answer = input(f"[{done + 1}/{total}] {token}  score 1-5, r replay, n note, s skip, q quit: ")
        except EOFError:
            return "quit", None
        answer = answer.strip().lower()
        if answer in {"1", "2", "3", "4", "5"}:
            return int(answer), note
        if answer == "r":
            play(player, wav)
        elif answer == "n":
            note = input("  note: ").strip() or None
        elif answer == "s":
            return "skip", None
        elif answer == "q":
            return "quit", None


def label(entry):
    speaker = entry.get("speaker")
    return f"{entry['bank']}/{speaker}" if speaker else entry["bank"]


def print_results(audition, scores):
    key_path = audition / "key.json"
    if not key_path.exists():
        sys.exit(f"{key_path} not found; cannot reveal")
    key = json.loads(key_path.read_text())
    rows = []
    for token, s in scores.items():
        entry = key.get(token)
        if entry:
            rows.append((s["score"], label(entry), entry["bank"], s.get("note")))
    rows.sort(key=lambda r: (-r[0], r[1]))
    print("\nPer voice (best first):")
    for score, name, _, note in rows:
        print(f"  {score}  {name:28}{'  — ' + note if note else ''}")
    banks = {}
    for score, _, bank, _ in rows:
        banks.setdefault(bank, []).append(score)
    print("\nPer bank (mean score, voices):")
    for bank, vals in sorted(banks.items(), key=lambda kv: -sum(kv[1]) / len(kv[1])):
        print(f"  {sum(vals) / len(vals):.2f}  {bank:16} ({len(vals)})")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("dir", nargs="?", default=str(REPO_ROOT / "target" / "audition"))
    ap.add_argument("--rescore", action="store_true", help="forget earlier scores")
    ap.add_argument("--results", action="store_true", help="only print the results")
    args = ap.parse_args()

    audition = Path(args.dir).expanduser().resolve()
    wavs = {p.stem: p for p in audition.glob("*.wav")}
    if not wavs:
        sys.exit(f"no WAVs in {audition}; run scripts/vocal-audition.py first")
    scores_path = audition / "scores.json"
    scores = {} if args.rescore else load_scores(scores_path)
    scores = {t: s for t, s in scores.items() if t in wavs}

    if not args.results:
        player = find_player()
        todo = [t for t in wavs if t not in scores]
        random.shuffle(todo)
        total = len(wavs)
        while todo:
            skipped = []
            for token in todo:
                print()
                play(player, wavs[token])
                result, note = ask(token, len(scores), total, player, wavs[token])
                if result == "quit":
                    print(f"stopped; {len(scores)}/{total} scored in {scores_path}")
                    return
                if result == "skip":
                    skipped.append(token)
                    continue
                scores[token] = {"score": result, **({"note": note} if note else {})}
                save_scores(scores_path, scores)
            if skipped and skipped == todo:
                print(f"\nonly skipped files left ({len(skipped)}); asking again")
            todo = skipped

    if len(scores) < len(wavs):
        print(f"{len(scores)}/{len(wavs)} scored; results stay hidden until all are")
        return
    print_results(audition, scores)


if __name__ == "__main__":
    main()
