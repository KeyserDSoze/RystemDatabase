# Rystem Database

**Hybrid document + relational + analytical database, built in Rust.**

> **Status: experimental preview.** The native CLI now supports persistent document put/get/delete using typed binary storage. **The network server remains health-only.** No indexed queries, SQL, multi-document transactions, MVCC, enforced foreign keys or production guarantees. The older `v0.1.1` release is health-only; verify [Releases](https://github.com/KeyserDSoze/RystemDatabase/releases) for the newer CLI preview before installing.

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

## Native document CLI (source on main)

```sh
printf '{"name":"Ada","address":{"city":"Roma"},"active":true}' |
  cargo run -p rystemdb-cli -- document put --data ./rystem-data --collection users --id u42 --json -

cargo run -p rystemdb-cli -- document get --data ./rystem-data --collection users --id u42
cargo run -p rystemdb-cli -- document delete --data ./rystem-data --collection users --id u42
```

Every CLI invocation opens the same on-disk binary segments with an exclusive writer lock; the CLI is not a network database API. Use [document CLI specification](docs/DOCUMENT_CLI.md) for typed references and byte values.

## Embedded storage (Rust library)

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

- [GitHub Releases](https://github.com/KeyserDSoze/RystemDatabase/releases): native Linux x64/arm64, macOS x64/arm64, Windows x64 binaries and SHA-256 checksums. `v0.1.1` is health-only; the planned `v0.1.2` is the first document CRUD CLI preview once its release workflow is confirmed.
- Bash installer: `scripts/install.sh` (Linux/macOS).
- PowerShell installer: `scripts/install.ps1` (Windows x64).
- npm/npx: `npx --yes rystemdb --version` once package name ownership and registry publication are established.
- Container: `docker compose -f docker/compose.yml up --build` with a persistent Docker volume mounted at `/data`; document CLI accessible via `docker compose exec` or separate `docker run` commands.

See [installation guidance](docs/INSTALL.md), [engineering workflow](docs/DEVELOPMENT.md), [roadmap](docs/ROADMAP.md), and [ADR 0001](adr/0001-foundation.md).

## Collaboration

We work **directly on `main`**, with GitHub Issues for bugs/features/chores and **RED → GREEN → REFACTOR** test-driven development. Every push executes cross-platform Cargo tests, linting, npm tests and a Docker build. Completion requires green CI for the final commit.

License: MIT.
