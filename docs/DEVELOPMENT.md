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

The native CLI stores binary documents; HTTP `serve` remains health-only. Experimental `rpc-serve` on main serves binary CRUD over **localhost only**, with optional token/ACL and TLS1.3 certificate-verified modes, and is **not in existing v0.1.2**. Never deploy it as a production network database server.

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

Wire/typed codec tests and EF scaffold are separate from **real Rust-process network SDK interoperability** run in the Linux/macOS/Windows .NET and Node CI jobs. No externally available authenticated server is implemented or released.

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

## Real .NET and TypeScript localhost RPC client tests (on main)

```sh
cargo build --locked -p rystemdb-cli
dotnet run --project src/libraries/dotnet/RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release -- target/debug/rystemdb
RYSTEMDB_BINARY="$(pwd)/target/debug/rystemdb" bash -c 'cd src/libraries/typescript/rystemdb-client && npm test'
```

These spawn a real native `rpc-serve --listen 127.0.0.1:0` subprocess, test typed HELLO/PING/CRUD, concurrent requests and restart recovery. Neither client permits non-loopback connections; these are **not** secure remote SDKs. See [LOCAL_CLIENTS.md](LOCAL_CLIENTS.md).

## Live .NET and Node RPC cross-OS CI gate (#48)

The [CI workflow](../.github/workflows/ci.yml) now runs the .NET 10 and Node.js 22 client suites **separately on ubuntu-latest, macos-latest, windows-latest**. Each builds the real Rust `rystemdb` binary, launches it with `--listen 127.0.0.1:0` from the client test process, verifies actual HELLO/PING/typed CRUD, competing writer exclusion and restart replay, then cleans up. Windows runners use the native `.exe`. The native installer/package release remains independent and is not a released RPC SDK.

## Optional local-only authentication and collection ACL TDD (#49/#50)

```sh
cargo test -p rystemdb-cli --locked --test rpc_auth
cargo test -p rystemdb-cli --locked --test rpc_auth_sdk
# CI also runs actual authenticated Rust-server interop in .NET 10 and Node 22 on all 3 OS.
```

Use [LOCAL_AUTH.md](LOCAL_AUTH.md) for strict private-file policies, HELLO token payload, ACL semantics and limitations. This is an **experimental plaintext localhost mode**, not TLS: no external binding or remotely safe authentication is available.

## Rust-only verified local TLS 1.3 regression (#51)

```sh
cargo test -p rystemdb-cli --locked --test rpc_tls
cargo test --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
```

[LOCAL_TLS.md](LOCAL_TLS.md) describes the opt-in TLS-only server with `--tls-cert`/`--tls-key`/`--auth-file`, CA+server-name verification in `LocalClient::connect_tls`, intentionally **public test certificates**, and why all nonloopback binds remain denied. The .NET 10 and Node 22 clients now also have separate CA/SAN-verified TLS1.3 APIs and test suites (#52); all remain localhost-only, not production remote transport.

## Cross-language TLS 1.3 source-only end-to-end gate (#52–54)

```sh
cargo build --locked -p rystemdb-cli
dotnet run --project src/libraries/dotnet/RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release -- target/debug/rystemdb
RYSTEMDB_BINARY="$(pwd)/target/debug/rystemdb" bash -c 'cd src/libraries/typescript/rystemdb-client && npm test'
```

CI runs the actual Rust server subprocess and TLS tests on Linux, macOS and Windows. On Windows use `target/debug/rystemdb.exe`; on macOS .NET 10 needs `DOTNET_SYSTEM_NET_SECURITY_USENETWORKFRAMEWORK=1` before process startup (the SDK also opts in) for TLS1.3. The regression suites reject wrong CA/hostname, expired and future certificates, malformed tokens and plaintext traffic on TLS-only ports. Server flushes RSWP status 6 before closing TLS (#54). Full instructions: [LOCAL_TLS.md](LOCAL_TLS.md). **#39 remains open for nonlocal production security.**

## Experimental localhost mTLS client-certificate + principal ACL (#58/#61)

```sh
cargo test --locked -p rystemdb-cli --test rpc_mtls
cargo test --locked -p rystemdb-cli --test rpc_mtls_policy
cargo test --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
```

The optional `--mtls-policy` and `--mtls-ca` flags together with TLS
certificate/key and bearer ACL start a localhost TLS1.3 listener requiring
certificate verification by an explicit client root CA. The verified leaf's
single canonical URI SAN is mapped to a principal ACL, intersected with the
authenticated HELLO token ACL; unauthorized requests are rejected before
storage access. Real-process negative/positive tests run in `rpc_mtls_identity`
using ephemeral OpenSSL test-only keys. No external bind or production
revocation/rotation is available; the original TLS-only SDK modes remain.

## Rust SDK client certificate transport test (#60)

```sh
cargo test --locked -p rystemdb-cli --test rpc_mtls_sdk
```

The test runner needs `openssl` in `PATH` for ephemeral private client
certificate and CA generation; no client identity private key is committed.
It verifies a real mutual TLS1.3 handshake with a test-only Rustls verifier
and verifies that the real CLI rejects a correctly verified certificate
when its bearer token is invalid. `rpc_mtls_identity` tests authorized
and denied CRUD under intersected certificate/token ACLs.

```sh
cargo test --locked -p rystemdb-cli --test rpc_mtls_identity
```

## Strict SAN edge-case TDD (#62)

```sh
cargo test --locked -p rystemdb-cli --bin rystemdb mtls_san
cargo test --locked -p rystemdb-cli --test rpc_mtls_identity
```

A strict, bounded DER *structure* check on the already-Rustls-verified client
leaf rejects invalid/multiple SAN entries, rather than relying on WebPKI's
`valid_uri_names` iterator, which deliberately filters malformed URI bytes.
The tests generate real ephemeral client identities with raw SAN extension
values using OpenSSL (`openssl` must be on `PATH`) plus deterministic malformed
DER mutation vectors; no private client keys are committed. This remains
an experimental **loopback-only** feature, without remote security sign-off.
