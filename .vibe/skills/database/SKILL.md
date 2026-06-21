---
name: database
description: "Use when designing schemas, writing SQL, or creating migrations."
---

# Database

Design clear schemas and safe, additive migrations.

## Schema
- Sensible normalization; primary keys on every table; foreign keys for relationships.
- Index columns used in `WHERE`/`JOIN`/foreign keys.
- Store timestamps as ISO-8601 / UTC. Be explicit about `NULL` vs `NOT NULL`.

## Migrations
- **Additive only**: new migration files with `CREATE TABLE IF NOT EXISTS` / `ALTER TABLE ADD COLUMN`.
- **Never edit an already-applied migration** (checksum mismatch) and never drop data on an update.

## Queries
- Always use parameterized queries (bind values) — never string-interpolate input (SQL injection).
- Wrap multi-statement changes in a transaction.
- Use `EXPLAIN`/`EXPLAIN QUERY PLAN` to diagnose slow queries.
