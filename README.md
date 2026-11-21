# Rystem Database

**Hybrid document + relational + analytical database, built in Rust.**

> **Status: experimental preview.** The native CLI now supports persistent document put/get/delete using typed binary storage. **The HTTP server remains health-only; `rpc-serve` on current main is experimental localhost-only binary CRUD (not in published v0.1.2).** No indexed queries, SQL, multi-document transactions, MVCC, enforced foreign keys or production guarantees. The older `v0.1.1` release is health-only; verify [Releases](https://github.com/KeyserDSoze/RystemDatabase/releases) for the newer CLI preview before installing.

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

## Client libraries and distributed architecture (experimental)

The four SDK subprojects are under [`src/libraries/`](src/libraries/README.md): Rust, .NET 10, TypeScript and a separate EF Core provider project. Rust/.NET/TypeScript have byte-level RSWP framing, CRUD key-prefix and native RDB typed value codecs with TDD conformance tests; **a localhost-only native RPC server and an experimental synchronous Rust `LocalClient` exist on `main`, but there is no secure remote client connection or functioning EF Core provider yet**.

- [RSWP v1 binary protocol](docs/WIRE_PROTOCOL.md), [cross-language CRUD key codec](docs/CRUD_KEY_PROTOCOL.md) and [typed RDB v1 value codec](docs/RDB_TYPED_PROTOCOL.md): 24-byte versioned envelope, 64-bit correlation ID, bounded payload and exact Rust/.NET/TypeScript key vectors. [Local TCP RPC prototype](docs/RPC_SERVER.md) on main (not in v0.1.2) uses binary document CRUD over loopback only.
- [Distributed design](docs/DISTRIBUTED_ARCHITECTURE.md): shard routing/epochs, replicated WAL/quorum, regional asynchronous replication and failover proposals; **not implemented**.
- [Issues #23–#35](https://github.com/KeyserDSoze/RystemDatabase/issues/23): actual feature backlog for server, clients, EF, sharding, geo, adaptive indexing, sparse storage and analytical Arrow Flight SQL.

## Distribution

- [GitHub Releases](https://github.com/KeyserDSoze/RystemDatabase/releases): native Linux x64/arm64, macOS x64/arm64, Windows x64 binaries and SHA-256 checksums. `v0.1.1` is health-only; **`v0.1.2` is the published document CRUD CLI preview**. The `rpc-serve` source is not yet in a new release.
- Bash installer: `scripts/install.sh` (Linux/macOS).
- PowerShell installer: `scripts/install.ps1` (Windows x64).
- npm/npx: `npx --yes rystemdb --version` once package name ownership and registry publication are established.
- Container: `docker compose -f docker/compose.yml up --build` with a persistent Docker volume mounted at `/data`; document CLI accessible via `docker compose exec` or separate `docker run` commands.

See [installation guidance](docs/INSTALL.md), [engineering workflow](docs/DEVELOPMENT.md), [roadmap](docs/ROADMAP.md), and [ADR 0001](adr/0001-foundation.md).

## Collaboration

We work **directly on `main`**, with GitHub Issues for bugs/features/chores and **RED → GREEN → REFACTOR** test-driven development. Every push executes cross-platform Cargo tests, linting, npm tests and a Docker build. Completion requires green CI for the final commit.

License: MIT.
