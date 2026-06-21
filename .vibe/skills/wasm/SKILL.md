---
name: wasm
description: "Use when crossing the Rust <-> WASM <-> TypeScript boundary (wasm-bindgen / wasm-pack)."
---

# Rust <-> WASM <-> TypeScript

## Bindings
- Annotate exports with `#[wasm_bindgen]`; build with `wasm-pack build --target web` (or `bundler`).
- Use `serde-wasm-bindgen` for passing structs/enums across the boundary; generate `.d.ts` types.
- Install `console_error_panic_hook` so Rust panics surface in the browser console.

## Performance
- Boundary calls are costly — keep them coarse-grained; batch work, don't chatter in loops.
- Pass bulk data as typed arrays / slices rather than many small calls.
- Be mindful of copying across the linear-memory boundary; reuse buffers where possible.
