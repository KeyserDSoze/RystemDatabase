# Experimental local RSWP RPC server — binary document CRUD

First implemented in Rystem Database **source on `main`** (Issue [#37](https://github.com/KeyserDSoze/RystemDatabase/issues/37)). **Not included in the already-published `v0.1.2` binary**, whose release assets are immutable. Install the next version only once the release workflow publishes it and passes actual asset smoke.

## Start a loopback-only server

```sh
cargo run --locked -p rystemdb-cli -- rpc-serve \
  --data ./data \
  --listen 127.0.0.1:8790
```

`--listen` defaults to `127.0.0.1:8790`. Any non-loopback address (including `0.0.0.0`, `::`, and all external IPv4/IPv6 addresses) is **rejected before opening/creating the database**. The existing `rystemdb serve` command is still the separate HTTP health-only endpoint; it does **not** serve documents.

**WARNING: no TLS or authentication. Use only on trusted localhost for development.** Do not tunnel or port-forward it to external networks. TLS, authentication and authorization are mandatory before external access; tracked in [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

## Wire frame

Each request/response is an [RSWP v1 frame](WIRE_PROTOCOL.md): 24-byte fixed big-endian header followed by at most 16 MiB opaque bytes. Keep-alive can send consecutive frames. The server always responds using the incoming operation and 64-bit request ID.

Before anything else, the client sends `HELLO` (operation=1) with exactly two bytes `01 00`, meaning protocol v1.0. The successful response returns the same two bytes. A call made before HELLO, or invalid HELLO body, receives status `2` and then the connection closes. Invalid wire header/version/oversized body terminates the connection without interpreting that payload.

`PING` (operation=2) accepts **empty** payload and returns an empty success payload. The prototype does not yet negotiate API capabilities, authentication, schema version or topology.

## Binary document CRUD payload v1 — local experimental contract

For `PUT`/`GET`/`DELETE`, the request body starts with:

```text
collection_length: u16 big-endian         // 1..128
collection:         [ASCII; length]       // [A-Za-z0-9_-] only
id_length:         u16 big-endian         // nonzero
id:                [UTF-8; length]        // no Unicode control chars
[PUT only] document: RDB\0 typed binary document v1
```

Combined `collection + "/" + id` must not exceed 1024 UTF-8 bytes. For GET and DELETE, **no additional bytes are allowed**. PUT requires a complete encoded `Value::Object` (nested arrays, absent/null distinctions, references, arbitrary binary bytes); malformed codec inputs, arrays/scalars at the root and truncated frames are rejected. This is not JSON and does not reinterpret `$rystem` wrappers.

Example GET key `users/u42` begins with precisely:

```text
00 05 75 73 65 72 73 00 03 75 34 32
```

Response:
- PUT success: status `0`, empty payload (after storage sync succeeds).
- GET success: status `0`, payload is one complete typed `RDB\0` binary value; missing key status `1` and empty payload.
- DELETE success: status `0`, empty payload; missing key status `1` and empty payload.
- Bad request: status `2`, empty payload; storage/internal error: status `4`, empty payload.

Error frames never include file paths, segment contents or exception messages. There is no idempotency guarantee: on a write-time disconnect, clients must assume the outcome is **uncertain**, not blindly retry a non-idempotent mutation.

## Shared CRUD key prefix across Rust, C# and TypeScript

The byte-exact key-prefix contract is now implemented in a **shared Rust `rystemdb-protocol::crud` module**, reused by server/client, and in independent .NET/TypeScript libraries. See [CRUD_KEY_PROTOCOL.md](CRUD_KEY_PROTOCOL.md) for exact golden bytes, Unicode handling, truncation and length guards. Key prefixes **and nested typed RDB values** now have independent three-language codec golden tests ([RDB_TYPED_PROTOCOL.md](RDB_TYPED_PROTOCOL.md)); **no live .NET/TypeScript TCP connection implementations yet** (#27/#28/#43).

## Experimental Rust client connection

The Rust workspace package `src/libraries/rust/rystemdb-client` now exports `LocalClient`, a **synchronous**, nonpooled TCP client. It automatically exchanges a HELLO frame and sends native binary documents. Its `connect` method refuses non-loopback addresses before opening a socket. This SDK is source-only on `main` until published and must not be represented as a remote secure client.

```rust
use rystemdb_client::LocalClient;
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::net::SocketAddr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = LocalClient::connect("127.0.0.1:8790".parse::<SocketAddr>()?)?;
    client.ping()?;
    let mut fields = BTreeMap::new();
    fields.insert("name".into(), Value::String("Ada".into()));
    client.put("users", "u42", &Value::Object(fields))?;
    println!("{:?}", client.get("users", "u42")?);
    Ok(())
}
```

The client's request ID is incremented on the same open connection. It has 3-second I/O timeouts; an I/O error or unexpected response returns an explicit error. PUT/DELETE transport failures may leave the remote commit outcome *uncertain*, so retries cannot be assumed safe.

The SDK connection is tested against a **real subprocess**, with nested typed documents, missing keys, DELETE tombstones, and rejection of external addresses in `src/rystemdb-cli/tests/rpc_rust_sdk.rs` (#26).

## Concurrency and limits

The server owns **one** advisory-locked `DocumentStore` for its entire lifetime. Independent CLI/embedded processes cannot open the same data directory while it runs. All operations coordinate through an in-process mutex, with up to eight worker threads and 32 queued sockets. Connections have 3-second per-read/write timeouts and a 128-frame maximum. Excess queued sockets are dropped rather than spawning unlimited threads.

This is **single node / single writer**, not an MVCC or multi-instance cluster. Multiple TCP clients can send requests, but they contend for one storage mutex. True request multiplexing, async I/O, streaming, resource quotas, TLS, authentication, cross-process reads, shard placement and replication are not implemented.

## TDD verification

`src/rystemdb-cli/tests/rpc_server.rs` starts **real `rystemdb rpc-serve` subprocesses** on ephemeral `127.0.0.1` ports. It tests HELLO/PING/keepalive/request IDs, nested binary document PUT/GET/DELETE/restart, rejected malformed bodies and versions, cross-process writer exclusivity, concurrent TCP clients and rejection of non-loopback listeners.

The Rust client is verified by a second RED→GREEN TDD cycle (#26). The initial server RED commit and all relevant final GREEN CI runs are recorded in [#37](https://github.com/KeyserDSoze/RystemDatabase/issues/37). This validates a loopback RPC prototype, **not** an authenticated production server. See [#25](https://github.com/KeyserDSoze/RystemDatabase/issues/25), [#38](https://github.com/KeyserDSoze/RystemDatabase/issues/38), and [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39) for follow-up gates.
