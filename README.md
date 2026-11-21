# Rystem Database

Rystem Database is an experimental Rust database engine combining document, relational and analytical workloads.

> **Development status:** early foundation. No persistent database, indexing, SQL, replication, or production-grade guarantees yet.

This repository follows the top-level organization of [TemporalMark](https://github.com/KeyserDSoze/TemporalMark): `src/`, `tests/`, `docs/`, `adr/`, `scripts/`, `docker/`, `infra/`, and `.github/`.

We develop on `main` using test-driven development (RED → GREEN → REFACTOR). GitHub Issues track all features, bugs and chores. Every push is checked by GitHub Actions.

See `docs/ROADMAP.md` and `adr/0001-foundation.md` for the initial scope and architecture.
