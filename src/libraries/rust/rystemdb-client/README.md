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

**Limits**: no TLS, credentials, authorization, async pool, routing, cancellation, server discovery or reliable automatic retries. Any write-time timeout has an uncertain outcome and must be reconciled. This is a localhost-only developer experiment; no working .NET or TypeScript client connections yet.

Only the `v0.1.2` *local document CLI* is released. `rpc-serve`/Rust `LocalClient` on main are not yet packaged in published release artifacts.

Protocol details: [docs/RPC_SERVER.md](../../../../docs/RPC_SERVER.md), [docs/WIRE_PROTOCOL.md](../../../../docs/WIRE_PROTOCOL.md).
