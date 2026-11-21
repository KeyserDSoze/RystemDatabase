# ADR 0001 — Rust foundation, repository topology and release gates

Status: Accepted — 2026-11-21

## Context
Rystem Database will target mixed point queries, relational references and analytical queries on nested dynamic documents. Development happens directly on main. We want repeatable tests and multi-platform native distribution.

## Decision
- Follow TemporalMark's top-level organization: src, tests, docs, adr, scripts, docker, infra and .github.
- Use Rust 2021 workspace with a first-party, dependency-free codec and CLI as an initial testable vertical slice.
- Keep the engine library independent of the optional network server.
- Represent nested values and references natively; the v1 codec is self-describing, not yet dictionary-compressed.
- Use TDD (RED/GREEN/REFACTOR) and run contract tests on every main push; do not consider a feature complete with a failing gate.
- Publish verified native binaries, checksums, a versioned container and, subject to npm ownership/credentials, an npx launcher.
- Do not claim database persistence, SQL, indexing, ACID, MVCC, or production readiness until separately implemented and verified.

## Next architectural gates
1. Crash-safe append-only row segments and durable catalog/index integration (candidate redb vs RocksDB, measured).
2. Atomic index/reference updates, snapshots and crash-injection tests.
3. DataFusion + Arrow + Parquet read paths with snapshot-aware overlays.
4. Cross-platform release installation, upgrade and rollback tests.

## Consequences
A minimal, testable, installable executable is possible before the complete engine. It is intentionally just a foundation health server. Release automation alone does not imply a published release.
