# rystemdb-client (Rust SDK prototype)

[Issue #26](https://github.com/KeyserDSoze/RystemDatabase/issues/26).

**Implemented on source main**: `LocalClient` with a single persistent loopback TCP connection to the experimental `rystemdb rpc-serve` process. It exchanges RSWP v1.0 HELLO, PING, PUT, GET and DELETE, with signed/typed nested binary document encoding via `rystemdb-core`.

```rust
use rystemdb_client::LocalClient;
use rystemdb_core::Value;
use std::net::SocketAddr;
use std::collections::BTreeMap;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = LocalClient::connect("127.0.0.1:8790".parse::<SocketAddr>()?)?;
    client.ping()?;
    let mut object = BTreeMap::new();
    object.insert("answer".into(), Value::Integer(42));
    client.put("docs", "one", &Value::Object(object))?;
    assert!(client.get("docs", "one")?.is_some());
    Ok(())
}
```

Tests: `cargo test -p rystemdb-client --locked` for key vectors and refusal of remote IPs, and `cargo test -p rystemdb-cli --test rpc_rust_sdk --locked` for separate real server subprocess interoperability across Linux/macOS/Windows.

**Limits**: optional local-only `LocalClient::connect_with_token(addr, &token)` supports a 32-byte HELLO bearer token and server-side collection ACLs; this is **not TLS, secure remote authentication, or a production credential lifecycle**. No async pool, routing, cancellation, discovery or reliable automatic retries. Write-time failure has an uncertain outcome. .NET and TypeScript also have working localhost-only clients, tested across Linux/macOS/Windows.

Only the `v0.1.2` *local document CLI* is released. `rpc-serve`/Rust `LocalClient` on main are not yet packaged in published release artifacts.

Protocol details: [docs/RPC_SERVER.md](../../../../docs/RPC_SERVER.md), [docs/WIRE_PROTOCOL.md](../../../../docs/WIRE_PROTOCOL.md).

## Shared binary key protocol

The Rust client uses `rystemdb-protocol::crud::encode_key` for requests (the native server uses the matching borrowed decoder). There are no duplicated collection/ID validators in the server and client. See [CRUD_KEY_PROTOCOL.md](../../../../docs/CRUD_KEY_PROTOCOL.md). This is still a localhost-only synchronous client.

## Opt-in local authentication

`LocalClient::connect_with_token(addr, &token)` sends the documented 36-byte authenticated HELLO. Read the raw `[u8; 32]` from a private client credential file, never put tokens in argv or logs. Wrong tokens and forbidden collection operations return `ClientError::Rejected(6)`. The server must be started with `--auth-file`. See [LOCAL_AUTH.md](../../../../docs/LOCAL_AUTH.md). **No TLS, rotation or remote access.**

## Verified TLS 1.3 on localhost (source preview)

The Rust client now exposes `LocalClient::connect_tls(addr, "localhost", trusted_ca_path, &raw32_token)`, which builds a TLS1.3-only session with **strict CA chain and DNS SAN verification** before sending the token or any RSWP frame. It works only with `rpc-serve --tls-cert ... --tls-key ... --auth-file ...`; plaintext connection methods never silently upgrade. Actual certificate validation, wrong-CA/wrong-name refusal and typed CRUD are exercised in `cargo test -p rystemdb-cli --test rpc_tls` on all 3 OS. See [LOCAL_TLS.md](../../../../docs/LOCAL_TLS.md). No secure remote bind, production mTLS, async pool or released SDK.

## Explicit local mTLS client certificate transport (#60/#61)

The Rust SDK now has a separate
`LocalClient::connect_mtls(addr, server_name, server_ca_path,
client_cert_path, client_key_path, &token)` API. This is a **TLS1.3-only,
numeric-localhost-only** certificate transport that validates the server
certificate against a user-supplied CA and verified DNS SAN, and presents the
client certificate/private key using rustls `with_client_auth_cert`. The
standard crypto provider checks certificate/key consistency. Neither
plaintext fallback nor insecure server certificate verification is permitted.

All three file paths must name regular nonsymlink files of at most **64 KiB**.
On Unix the client private key must have no group/other permissions (for
example `0600`); the loader also detects replacement between metadata
inspection and opening. **Windows NTFS ACL checks are not implemented**;
restrict key permissions yourself and do not assume production credential
security. TLS key contents, filenames and bearer-token bytes are never logged.

The real `rpc-serve --mtls-policy ... --mtls-ca ...` listener now
permits HELLO and CRUD **only when** a trusted client leaf contains exactly
one canonical, exact-match `spiffe://rystemdb.local/client/<id>` URI SAN
and **no additional DNS/IP/URI/other SAN entries**, enforced by the
bounded strict DER guard on a Rustls-verified client leaf (#62), whose
principal ACL grants intersect with a valid, independent 32-byte bearer
token ACL. A read-only certificate cannot gain write permission from an RW
token; unknown principals, wrong issuers/EKU and wrong tokens are denied.
`rpc_mtls_identity` verifies this using a real Rust server and freshly
generated OpenSSL client certificates. `rpc_mtls_sdk` covers Rustls transport
and wrong token denial. No private test client keys are committed.

This is a **source-only experimental localhost** SDK, not production remote
mTLS, and is not released as a package. Revocation, Windows ACL audit, .NET/
Node mTLS client-cert support, rotation and rate limiting remain open.
