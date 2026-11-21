# Development: TDD on main

1. Open a GitHub Issue (feature, bug, or chore). Record an observable behavior and acceptance criteria.
2. **RED:** add a failing test for the specific behavior first. Record the test command/output in the issue. A transient red commit on main is acceptable, but do not release from it.
3. **GREEN:** implement the smallest change that passes the test. Verify all unit/integration tests and affected interfaces.
4. **REFACTOR:** clean up while preserving green tests. Update docs and the relevant ADR for nontrivial decisions.
5. Only close the issue when the final main SHA is green on GitHub CI. The project has no PR-only branch workflow.

Local requirements: Rust 1.89.0+, Cargo, Node.js 20+, optionally Docker.

```sh
cargo test --workspace --locked --all-targets
cargo test -p rystemdb-storage --locked --features fault-injection --tests -- --test-threads=1
cargo clippy --workspace --locked --all-targets --all-features -- -D warnings
cd npm/cli && npm test
docker build -f docker/Dockerfile -t rystemdb:local .
```

Checks now cover the nested binary codec, persistent document CLI, append-only storage, process crash simulations, and synthetic I/O errors. An advisory cross-process writer lock is provided; **no transactional secondary indexes, multi-document ACID, enforced reference integrity, or proven power-loss durability exist**. Each future feature must introduce RED tests. The Windows/macOS/Linux checks cannot be replaced by Linux-only CI.

Native build: `cargo build --release --locked -p rystemdb-cli`. Run `./target/release/rystemdb --help`. On Windows use `target\\release\\rystemdb.exe`.

## Releasing

A main push automatically publishes a prerelease only if the semver release corresponding to `npm/cli/package.json` does not already exist; later main pushes at the same version do not replace immutable assets. A matching `vX.Y.Z` tag can also launch the workflow. The release job verifies tests, builds native files, uploads SHA256SUMS to GitHub Releases and publishes a GHCR image. Bump Cargo workspace and npm package versions together before the next version. npm publishing needs `NPM_TOKEN` and ownership of the `rystemdb` package on npm; without the token the package step is intentionally skipped. Check release asset smoke tests before advertising an installer.

The native CLI stores binary documents; the HTTP `serve` endpoint remains health-only. A separate experimental `rpc-serve` on main serves binary CRUD over **localhost only** with no TLS/auth and is **not in the existing v0.1.2 release**. Never deploy it as a production network database server.

## Crash-injection safety

`fault-injection` is a **test-only Cargo feature**, disabled in normal/release builds. Test children can exit without unwinding and are expected to leave incomplete file frames for replay. Never ship or package production binaries with that feature. A green process-crash matrix is not a proof of physical power-loss persistence; the [storage safety issues](https://github.com/KeyserDSoze/RystemDatabase/issues/17) remain open for this and other guarantees.

## Protocol conformance and SDK scaffolds

```sh
cargo test -p rystemdb-protocol --locked --all-targets
cargo test -p rystemdb-client --locked --all-targets
cd src/libraries/typescript/rystemdb-client && npm test && cd ../../../..
dotnet run --project src/libraries/dotnet/RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release
dotnet build src/libraries/dotnet/RystemDatabase.EntityFrameworkCore/RystemDatabase.EntityFrameworkCore.csproj --configuration Release
```

These commands only validate binary frame codecs and EF build scaffolding; no remote SDK/server integration is yet implemented. All tests must run in CI on main; do not ship unauthenticated RPC servers.

## Local-only binary RPC test (unreleased on main)

```sh
cargo test -p rystemdb-cli --locked --test rpc_server -- --test-threads=1
cargo run -p rystemdb-cli -- rpc-serve --data ./data --listen 127.0.0.1:8790
```

The server requires HELLO v1.0 and uses RSWP frames with typed binary RDB values, not JSON; see [RPC_SERVER.md](RPC_SERVER.md). Never bind a non-loopback address or tunnel the unauthenticated prototype remotely. Future TLS/auth is tracked in #39.

## Rust RPC client TDD (source on main)

```sh
cargo test -p rystemdb-cli --locked --test rpc_rust_sdk
cargo test -p rystemdb-client --locked --all-targets
```

The Rust `LocalClient` is synchronous and loopback-only, **not** an authenticated remote or distributed client. See [RPC_SERVER.md](RPC_SERVER.md) and #26.

## Cross-language native typed RDB conformance (source on main)

```sh
cargo test -p rystemdb-core --locked --test rdb_compat
cd src/libraries/typescript/rystemdb-client && npm test
dotnet run --project src/libraries/dotnet/RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release
```

The C#/TypeScript RDB codecs can **encode and decode** nested binary objects but are not network database clients. [RDB_TYPED_PROTOCOL.md](RDB_TYPED_PROTOCOL.md) documents type mapping and binary compatibility. TDD race/UTF-8 defect #44 requires preserving the U+FEFF scalar.
