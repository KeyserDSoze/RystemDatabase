# Development: TDD on main

1. Open a GitHub Issue (feature, bug, or chore). Record an observable behavior and acceptance criteria.
2. **RED:** add a failing test for the specific behavior first. Record the test command/output in the issue. A transient red commit on main is acceptable, but do not release from it.
3. **GREEN:** implement the smallest change that passes the test. Verify all unit/integration tests and affected interfaces.
4. **REFACTOR:** clean up while preserving green tests. Update docs and the relevant ADR for nontrivial decisions.
5. Only close the issue when the final main SHA is green on GitHub CI. The project has no PR-only branch workflow.

Local requirements: Rust 1.89.0+, Cargo, Node.js 20+, optionally Docker.

```sh
cargo test --workspace --locked --all-targets
cargo test -p rystemdb-storage --locked --features fault-injection --test crash_injection -- --test-threads=1
cargo clippy --workspace --locked --all-targets --all-features -- -D warnings
cd npm/cli && npm test
docker build -f docker/Dockerfile -t rystemdb:local .
```

Checks currently exercise the codec and CLI foundation only. The project now has experimental append-only persistent document segments and contract tests, but **an advisory cross-process writer lock, but **no transactional index, multi-document transaction or production-grade crash/power-loss guarantees****; each future feature must introduce RED tests. The Windows/macOS/Linux checks cannot be replaced by Linux-only CI.

Native build: `cargo build --release --locked -p rystemdb-cli`. Run `./target/release/rystemdb --help`. On Windows use `target\\release\\rystemdb.exe`.

## Releasing

A main push automatically publishes a prerelease only if the semver release corresponding to `npm/cli/package.json` does not already exist; later main pushes at the same version do not replace immutable assets. A matching `vX.Y.Z` tag can also launch the workflow. The release job verifies tests, builds native files, uploads SHA256SUMS to GitHub Releases and publishes a GHCR image. Bump Cargo workspace and npm package versions together before the next version. npm publishing needs `NPM_TOKEN` and ownership of the `rystemdb` package on npm; without the token the package step is intentionally skipped. Check release asset smoke tests before advertising an installer.

This foundation cannot store data; do not use the health server as a deployed database.

## Crash-injection safety

`fault-injection` is a **test-only Cargo feature**, disabled in normal/release builds. Test children can exit without unwinding and are expected to leave incomplete file frames for replay. Never ship or package production binaries with that feature. A green process-crash matrix is not a proof of physical power-loss persistence; the [storage safety issues](https://github.com/KeyserDSoze/RystemDatabase/issues/17) remain open for this and other guarantees.
