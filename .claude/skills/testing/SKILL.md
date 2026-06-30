---
name: testing
description: "Use when writing or fixing tests, or before marking work done."
---

# Testing

## Write good tests
- Test observable behavior and contracts, not private implementation details.
- Arrange–Act–Assert; cover happy path, edge cases, and error paths.
- Keep tests deterministic — no real time, randomness, or network unless that is the thing under test.

## Before marking work done
- Run the project's full test suite; everything must pass.
- Fix flakes rather than retrying. Rust: `cargo test`. TS: `vitest`/`jest`.
- Never mark a ba todo `done` unless the change is implemented AND tests pass.
