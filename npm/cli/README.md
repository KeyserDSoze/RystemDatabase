# rystemdb (npm CLI)

Native binary launcher for Rystem Database. Once the npm package and GitHub release have been published:

```sh
npx --yes rystemdb --version
npx --yes rystemdb serve
```

Requires Node.js 20+. The launcher downloads the matching native binary for the package version from GitHub Releases, verifies its SHA-256 against the release checksum manifest, caches it and executes it. No Rust toolchain is needed on client machines.

This package is **not yet published**. The core is a foundation preview; it does not persist documents.
