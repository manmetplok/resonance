---
name: rust
description: "Use when writing or reviewing Rust — idioms, error handling, ownership, async, and tests."
---

# Rust

Write idiomatic, safe, well-tested Rust.

## Practices
- Errors: return `Result<T, E>`; use `thiserror` for libraries, `anyhow` for apps. Propagate with `?`. Avoid `unwrap()`/`expect()` outside tests and provably-infallible cases.
- Ownership: prefer borrows (`&T`/`&mut T`) over cloning; reach for `Clone`/`Arc` only when needed.
- Prefer iterators and combinators over manual index loops; use `match` and `if let`.
- Derive `Debug, Clone, PartialEq` etc. where cheap; implement `From`/`TryFrom` for conversions.
- Keep modules small; make items `pub` deliberately.
- Async: `tokio` runtime; don't block the executor; share state with `Arc<Mutex<…>>` sparingly.
- `unsafe` only when necessary and document every block with a `// SAFETY:` comment.

## Always
- `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` before considering work done.
