# TLS 1.3 on loopback — experimental Rust client/server preview

Implemented in source on `main` under [#51](https://github.com/KeyserDSoze/RystemDatabase/issues/51), a scoped part of [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

**This is real certificate-verified TLS 1.3 but NOT a remotely deployable database service.** The server and Rust client still reject every non-loopback address; mTLS, secure production identity/rotation/revocation, Windows ACL audit, rate limiting, cluster authorization, and TLS support in .NET/Node SDKs are not implemented. The published `v0.1.2` binaries do not contain `rpc-serve` or TLS.

## Source build and configuration

```bash
cargo build --locked -p rystemdb-cli

./target/debug/rystemdb rpc-serve \
  --data ./rystem-data \
  --listen 127.0.0.1:8790 \
  --auth-file ./private/acl.txt \
  --tls-cert ./private/server-cert.pem \
  --tls-key ./private/server-key.pem
```

TLS and authentication are **opt-in together**: `--tls-cert` and `--tls-key` must both be provided, and TLS mode **requires** `--auth-file` with explicit collection read/write grants. Missing, malformed, oversized or mismatched key/cert, absent ACL, loose Unix key permissions and symlinked PEM keys fail at startup, before creating storage files. The private key is only read from an explicit file path (not from process arguments or logs). The certificate chain can be publicly readable. The max PEM length is 64 KiB per file.

Use a **privately generated CA and server key** for development. The server cert must contain a Subject Alternative Name matching the name the client verifies (for example `DNS:localhost`); its private key must match the certificate. Protect key and ACL files with mode `0600` on Unix and restrictive NTFS ACLs on Windows. The checked-in `src/rystemdb-cli/tests/fixtures/tls` key is **PUBLIC and for automated tests only**, never for actual use. Do not install the test root certificate into a global trust store.

## Rust SDK: certificate chain and server name verification

```rust
use rystemdb_client::LocalClient;
use std::{fs, net::SocketAddr, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read("private/write.token")?;
    let token: [u8; 32] = bytes.try_into().map_err(|_| "wrong token size")?;
    let addr: SocketAddr = "127.0.0.1:8790".parse()?;

    // Connect only to numeric localhost, verify cert chain against the
    // supplied CA, and verify SAN DNS name "localhost".
    // The bearer token is sent ONLY AFTER the verified TLS 1.3 handshake.
    let mut db = LocalClient::connect_tls(
        addr, "localhost", Path::new("private/root-ca.pem"), &token,
    )?;
    db.ping()?;
    Ok(())
}
```

The client uses `rustls 0.23.46`, the ring provider, **only TLS 1.3**, a local CA trust store and strict SAN/name validation. There is no insecure custom verifier or plaintext fallback. A wrong root, wrong server name or TLS handshake failure is an error **before** token transmission. Within the authenticated TLS stream, native RSWP v1 framing, typed binary CRUD and collection ACL status codes remain unchanged.

`LocalClient::connect` and `LocalClient::connect_with_token` remain **plaintext localhost-only developer prototypes**. They cannot communicate with a TLS-only listener. The .NET 10 and TypeScript/Node SDKs currently also use plaintext localhost only, including their optional local-token forms; **do not connect them to this TLS listener** until independent verified TLS transport is implemented and tested.

## Security and negative tests

```bash
cargo test -p rystemdb-cli --test rpc_tls --locked
cargo test --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
```

The real-process integration suite starts the executable with `--listen 127.0.0.1:0` to avoid ephemeral port races and verifies TLS 1.3 Rust-client HELLO/PING/native typed CRUD, wrong trusted CA, wrong hostname, plaintext RSWP against the TLS port, missing/incorrect bearer tokens, malformed/mismatched key/certificate, insecure TLS config, and non-loopback bind denial. The same Cargo suite executes on Linux, macOS and Windows in CI.

**Important remaining risks:** Publicly exposed bind remains prohibited even with TLS; no mTLS, no cluster admission control, no certificate expiry/rotation/revocation workflow, no Windows ACL validation, no per-user rate limits, no full parser fuzzing or comprehensive remote threat model. Compromised local credentials and on-disk files are outside this transport layer's protection. Complete [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39) before opening to other machines. [RSWP protocol](WIRE_PROTOCOL.md) | [Local bearer-token policy](LOCAL_AUTH.md) | [Local SDK guide](LOCAL_CLIENTS.md).
