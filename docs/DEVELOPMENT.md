# Development: TDD on main

1. Open a GitHub Issue (feature, bug, or chore). Record an observable behavior and acceptance criteria.
2. **RED:** add a failing test for the specific behavior first. Record the test command/output in the issue. A transient red commit on main is acceptable, but do not release from it.
3. **GREEN:** implement the smallest change that passes the test. Verify all unit/integration tests and affected interfaces.
4. **REFACTOR:** clean up while preserving green tests. Update docs and the relevant ADR for nontrivial decisions.
5. Only close the issue when the final main SHA is green on GitHub CI. The project has no PR-only branch workflow.

Local requirements: Rust 1.85.1+, Cargo, Node.js 20+, optionally Docker.

```sh
cargo test --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
cd npm/cli && npm test
docker build -f docker/Dockerfile -t rystemdb:local .
```

Checks currently exercise the codec and CLI foundation only. The project does **not** yet have a persistent database or transaction tests; each future feature must introduce its own RED tests. The Windows/macOS/Linux checks cannot be replaced by Linux-only CI.

Native build: `cargo build --release --locked -p rystemdb-cli`. Run `./target/release/rystemdb --help`. On Windows use `target\\release\\rystemdb.exe`.

## Releasing

A main push automatically publishes a prerelease only if the semver release corresponding to `npm/cli/package.json` does not already exist; later main pushes at the same version do not replace immutable assets. A matching `vX.Y.Z` tag can also launch the workflow. The release job verifies tests, builds native files, uploads SHA256SUMS to GitHub Releases and publishes a GHCR image. Bump Cargo workspace and npm package versions together before the next version. npm publishing needs `NPM_TOKEN` and ownership of the `rystemdb` package on npm; without the token the package step is intentionally skipped. Check release asset smoke tests before advertising an installer.

This foundation cannot store data; do not use the health server as a deployed database.
