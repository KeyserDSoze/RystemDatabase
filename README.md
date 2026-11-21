# Rystem Database

**Hybrid document + relational + analytical database, built in Rust.**

> **Status: experimental foundation.** A nested typed binary codec, an embedded append-only document segment library and a minimal health-only CLI exist. **The server does not expose storage**, and there are no transactional indexes, SQL, MVCC or production guarantees. `v0.1.1` binaries predate the storage implementation; no new release is implied by ongoing `main` development.

Rystem Database follows the top-level structure of [TemporalMark](https://github.com/KeyserDSoze/TemporalMark): `src/`, `tests/`, `docs/`, `adr/`, `scripts/`, `docker/`, `infra/`, `.github/`.

## Build and test

Requirements: Rust 1.89.0+, Cargo. Node.js 20+ for launcher tests; Docker optional.

```sh
cargo test --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
cd npm/cli && npm test
cd ../..
cargo run -p rystemdb-cli -- --version
cargo run -p rystemdb-cli -- serve
# then in another terminal: curl http://127.0.0.1:8787/healthz
```

## Embedded storage (source on main; not in published CLI yet)

```rust
use rystemdb_core::Value;
use rystemdb_storage::DocumentStore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut db = DocumentStore::open("./rystem-data")?;
    db.put("users/42", &Value::String("example".into()))?;
    assert_eq!(db.get("users/42")?, Some(Value::String("example".into())));
    db.delete("users/42")?;
    Ok(())
}
```

See [storage format, crash-recovery tests and limits](docs/STORAGE.md). Feature-gated real-process crash tests and synthetic I/O failure/replay tests are exercised in CI; this remains a **single-process experimental API** with an OS advisory single-writer lock, but without transactional multi-document updates, secondary indexes or proven physical power-loss durability.

## Distribution

- [v0.1.1 GitHub prerelease](https://github.com/KeyserDSoze/RystemDatabase/releases/tag/v0.1.1): native Linux x64/arm64, macOS x64/arm64, Windows x64 binaries and SHA-256 checksums. These binaries are **health-only**, without the storage library linked.
- Bash installer: `scripts/install.sh` (Linux/macOS).
- PowerShell installer: `scripts/install.ps1` (Windows x64).
- npm/npx: `npx --yes rystemdb --version` once package name ownership and registry publication are established.
- Container: `docker compose -f docker/compose.yml up --build`, then versioned GHCR images after tagged builds.

See [installation guidance](docs/INSTALL.md), [engineering workflow](docs/DEVELOPMENT.md), [roadmap](docs/ROADMAP.md), and [ADR 0001](adr/0001-foundation.md).

## Collaboration

We work **directly on `main`**, with GitHub Issues for bugs/features/chores and **RED → GREEN → REFACTOR** test-driven development. Every push executes cross-platform Cargo tests, linting, npm tests and a Docker build. Completion requires green CI for the final commit.

License: MIT.
