# Third-party licenses (DEP-04)

Resonance is licensed under **MIT OR Apache-2.0** (`LICENSE-MIT`,
`LICENSE-APACHE`). Every member manifest inherits it with
`license.workspace = true`. This document lists the obligations that
third-party dependencies add on top of that. Read it before the app is
distributed to anyone but Jorrit.

## Copyleft and notice obligations found in the dependency tree

- **LAME (LGPL-3.0), statically linked.** `resonance-audio`'s default `mp3`
  feature pulls in `mp3lame-encoder`/`mp3lame-sys`, which builds LAME from
  source and links it statically into the app
  (`rustc-link-lib=static=mp3lame`). LGPL-3.0 permits static linking, but
  only if the end user can relink the app against a modified LAME — in
  practice that means either shipping the LAME object files (or the whole
  app's object files) so a recipient can relink, linking LAME dynamically
  instead, or shipping LAME's complete source alongside a build system that
  reproduces the static link. None of that is in place today.
- **symphonia and friends (MPL-2.0).** The ~13 `symphonia-*` crates
  (resonance-common's decode path, used by the app and by the IR/drums/
  mastering plugins) and `option-ext` are MPL-2.0. MPL requires that MPL-
  licensed *source files* stay available under MPL terms (unmodified files
  can simply be redistributed as part of the larger work; modified files
  must have their MPL source made available) and that a notice of the MPL
  license be reasonably discoverable. We don't modify symphonia's sources,
  so this is a notices-file obligation, not a source-release one.
- **self_cell (Apache-2.0 OR GPL-2.0-only).** Dual-licensed; nothing to do
  here beyond recording that we'd select Apache-2.0 under its terms once a
  notices file exists.
- Everything else surfaced in the DEP-04 review pass (MIT, Apache-2.0, BSD,
  Zlib, Unicode-3.0, ...) is permissive and imposes no action beyond
  attribution in a notices file.

## What's NOT done here

- **No LGPL relink story for LAME.** Static linking stays as-is; this is
  fine only because Jorrit is the sole user today (`project_no_real_users_yet`).
- **No generated third-party notices file.** `cargo about` or `cargo deny
  list` would generate one mechanically once a license is chosen; doing it
  by hand now would just go stale.

## Decision needed from Jorrit before any distribution

1. ~~Pick a project license~~: done, MIT OR Apache-2.0 (2026-10-04).
2. **Decide the LAME story**, which is still open: dynamic link (simplest LGPL compliance, but
   requires the system or bundle to ship `libmp3lame.so`/`.dylib` and the
   app to `dlopen`/link against it instead of the static build), or static
   link with a documented relink process (ship object files or a
   from-source build script a recipient can use to relink).
3. Once 1 and 2 are settled, generate and commit a real notices file (not
   this document) with `cargo about generate` or `cargo deny list
   --format json`, and wire a CI/pre-release check that it stays current.

LAME is still statically linked. MIT/Apache-2.0 for our own code does not
change the LGPL obligations on LAME.
