# rystemdb npm launcher

The npm package is **not yet published**. Registry name ownership and publishing credentials are tracked in [Issue #1](https://github.com/KeyserDSoze/RystemDatabase/issues/1).

After actual npm publication, the native launcher can be invoked with:

```sh
npx --yes rystemdb --version
npx --yes rystemdb document get --data ./data --collection users --id u42
```

Requires Node.js 20+. The JS wrapper downloads and verifies the matching SHA-256 native binary from GitHub Releases, caches it, then runs the **Rust** executable. It is not a JavaScript database implementation. The `v0.1.1` native binary only has health endpoints; document CLI requires `v0.1.2` or newer.

For the native commands, JSON typed reference/bytes contract, and security limits see [DOCUMENT_CLI.md](../../docs/DOCUMENT_CLI.md). The server remains health-only.
