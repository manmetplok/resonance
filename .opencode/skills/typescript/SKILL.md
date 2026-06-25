---
name: typescript
description: "Use when writing TypeScript / React frontend code."
---

# TypeScript / React

## TypeScript
- `strict` mode on; type the public boundaries explicitly; avoid `any` (use `unknown` + narrowing).
- Model variants with discriminated unions; handle every case.
- `async`/`await` with explicit error handling.

## React
- Follow the rules of hooks; keep components pure and small; derive state instead of duplicating it.
- Give list items stable `key`s; handle loading/error/empty states for data fetching.
- Lift state only as far as needed; prefer composition over deep prop drilling.

## Always
- Run `tsc --noEmit` and the linter before done.
