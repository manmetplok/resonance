# Resonance — collaboration notes

Work in this repo is driven through the `ba-*` agents
(`.claude/agents/ba-*.md`): `ba-architect` scopes todos, `ba-developer`
implements them, `ba-planner` triages the backlog, `ba-researcher`
gathers external knowledge, and `ba-reverse-engineer` keeps the platform
registry in sync.

## Tests

Run the suite with `./scripts/run-tests.py` (add `-p <crate>` to narrow it).
Plain `cargo test` works too, but it walks its ~390 test binaries one at a
time; the script builds with cargo and runs them concurrently, which is 24s
against 157s for the same tests. It also launches each binary from its own
crate root, which the golden-image tests need.

Three things to know before adding tests (background in ba doc #285):

- **Build the app with `Resonance::new_for_test()`**, not `Resonance::new()`.
  The real constructor opens an audio stream, probes devices, loads whatever
  CLAP plugins are installed on the machine, and reads the user's config — a
  third-party plugin's broken teardown used to abort test processes at random
  because of it. Use `new_for_test_on(tab)` to start on a given tab, and
  `new_for_test_with_capture()` to assert on emitted engine commands.
- **Don't add a new file to `resonance-app/tests/`.** Add a module to one of
  the group binaries (`mixer`, `timeline`, `compose`, `control`, …) instead.
  Every extra target re-monomorphizes the whole app plus iced, which is why
  240 of them cost 193s to relink after a one-line change and 11 cost 8s.
- **Re-bless goldens with `RESONANCE_BLESS=1`**, e.g.
  `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer`. This machine is
  canonical for golden images.
