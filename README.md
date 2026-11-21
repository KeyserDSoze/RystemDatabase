# Rystem Database

**Hybrid document + relational + analytical database, built in Rust.**

> **Status: foundation preview only.** A nested typed binary document codec and minimal health-only CLI exist. There is **no durable storage, data API, indexes, SQL, MVCC, or production deployment** yet. Packaging workflows are prepared; a main push automatically attempts a one-time prerelease for a new version. Publication must be independently verified.

Rystem Database follows the top-level structure of [TemporalMark](https://github.com/KeyserDSoze/TemporalMark): `src/`, `tests/`, `docs/`, `adr/`, `scripts/`, `docker/`, `infra/`, `.github/`.

## Build and test

Requirements: Rust 1.85.1+, Cargo. Node.js 20+ for launcher tests; Docker optional.

```sh
cargo test --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
cd npm/cli && npm test
cd ../..
cargo run -p rystemdb-cli -- --version
cargo run -p rystemdb-cli -- serve
# then in another terminal: curl http://127.0.0.1:8787/healthz
```

## Planned distribution (available *only after* actual publication)

- GitHub Releases: native Linux x64/arm64, macOS x64/arm64, Windows x64 binaries and SHA-256 checksums.
- Bash installer: `scripts/install.sh` (Linux/macOS).
- PowerShell installer: `scripts/install.ps1` (Windows x64).
- npm/npx: `npx --yes rystemdb --version` once package name ownership and registry publication are established.
- Container: `docker compose -f docker/compose.yml up --build`, then versioned GHCR images after tagged builds.

See [installation guidance](docs/INSTALL.md), [engineering workflow](docs/DEVELOPMENT.md), [roadmap](docs/ROADMAP.md), and [ADR 0001](adr/0001-foundation.md).

## Collaboration

We work **directly on `main`**, with GitHub Issues for bugs/features/chores and **RED → GREEN → REFACTOR** test-driven development. Every push executes cross-platform Cargo tests, linting, npm tests and a Docker build. Completion requires green CI for the final commit.

License: MIT.
