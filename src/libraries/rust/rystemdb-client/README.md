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
