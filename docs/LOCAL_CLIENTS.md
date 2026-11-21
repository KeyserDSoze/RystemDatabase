# Rystem Database — experimental local client connections

This page describes the **source-only localhost TCP client milestone**, developed directly on main under [#45](https://github.com/KeyserDSoze/RystemDatabase/issues/45), [#46](https://github.com/KeyserDSoze/RystemDatabase/issues/46), and parent [#27](https://github.com/KeyserDSoze/RystemDatabase/issues/27)/[#28](https://github.com/KeyserDSoze/RystemDatabase/issues/28).

**Security warning:** the Rust RSWP RPC server does **not implement secure remote authentication or public network policy**. An **optional bearer-token HELLO plus local collection ACL** is available ([LOCAL_AUTH.md](LOCAL_AUTH.md)). The **Rust server/Rust SDK** now also implement [certificate-verified TLS 1.3 on localhost](LOCAL_TLS.md); **.NET and Node clients still use plaintext local TCP** and must not be treated as remote-safe. It will only bind numeric loopback. All three SDKs reject non-loopback connection targets. Do not use port-forwarding or public reverse proxies to bypass that restriction. A production remote client requires [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

**Release warning:** the current published native release `v0.1.2` predates this RPC server and these SDK connection implementations. Build `rystemdb` from main to run this example. None of the .NET/TypeScript client packages is published to NuGet/npm at this point; npm/cli is an independent native-binary launcher.

## Run a real Rust server

```sh
cargo build --locked -p rystemdb-cli
./target/debug/rystemdb rpc-serve --data ./rystem-data --listen 127.0.0.1:8790
```

The server owns the exclusive binary segment writer lock for its entire process lifetime; you cannot concurrently open a second local CLI writer on that directory.

## C#/.NET 10 (asynchronous, one persistent socket)

Use the project in `src/libraries/dotnet/RystemDatabase.Client`, or add it as a project reference to your own solution.

```csharp
using System.Net;
using RystemDatabase.Client;

await using var db = await RystemLoopbackClient.ConnectAsync(
    IPAddress.Loopback, 8790);
await db.PingAsync();
var doc = new RdbValue.Object(new Dictionary<string, RdbValue> {
    ["name"] = new RdbValue.Text("Ada"),
    ["age"] = new RdbValue.Integer(34),
    ["data"] = new RdbValue.Bytes(new byte[] { 0, 255, 128 })
});
await db.PutAsync("users", "u42", doc);
RdbValue.Object? loaded = await db.GetAsync("users", "u42");
bool deleted = await db.DeleteAsync("users", "u42");
```

`ConnectAsync` validates the numeric IP before socket creation and performs RSWP 1.0 HELLO. Async operations take optional `CancellationToken`, use 3-second per-call deadlines, validate the 24-byte frame and response ID/opcode, and serialize I/O through a semaphore. Cancellation or broken TCP cannot guarantee the outcome of a write; the connection closes rather than blindly replaying it. A missing GET becomes `null`, and a second DELETE yields `false`. Errors return explicit exceptions / `RystemRpcException.Status`.

**Not yet implemented**: TLS/auth, pooling, multiplexing, automatic safe idempotency/retries, query execution, shard-aware placement and NuGet distribution. Parent #27 remains open.

## TypeScript / Node.js 22+ (asynchronous, correlated concurrent requests)

Use `src/libraries/typescript/rystemdb-client/src/client.ts` alongside the existing `wire.ts`, `crud.ts`, `rdb.ts` from this source tree.

```typescript
import { RystemLocalClient } from "./src/client.ts";

const db = await RystemLocalClient.connect("127.0.0.1", 8790);
try {
  await db.ping();
  const doc = new Map([
    ["name", "Ada"],
    ["age", 34n],                  // signed i64: use bigint, not number
    ["data", Uint8Array.from([0, 255, 128])],
  ]);
  await db.put("users", "u42", doc);
  console.log(await db.get("users", "u42"));
  await db.delete("users", "u42");
} finally {
  await db.close();
}
```

`connect` permits only numeric `127.0.0.1` and `::1` (no DNS). A native socket `Buffer` is normalized to **plain `Uint8Array`**, with independent golden tests plus a real-process regression for a Buffer subclass bug [#47](https://github.com/KeyserDSoze/RystemDatabase/issues/47). The socket parser handles TCP frame fragmentation, guards 16 MiB per message, and correlates concurrent requests by native BigInt u64 IDs. A bounded pending-request map and 3-second inactivity/request timers fail closed on lost connection. No automatic write retries or full streaming queries.

**Not yet implemented**: TLS, authentication, user-provided AbortSignal, connection pools/retry policy, browser support, NuGet/npm SDK release, sharding or geo-redundancy. Parent #28 remains open.

## Real Rust-process CI verification (Linux, macOS, Windows)

These are **not mock tests**: each live test launches the Rust CLI as an actual OS subprocess, uses `--listen 127.0.0.1:0` to atomically select a port inside the server, parses the flushed startup line, exercises HELLO/PING/native nested binary PUT/GET/DELETE and a true process restart, then closes all processes.

```sh
cargo build --locked -p rystemdb-cli
# On Windows pass target/debug/rystemdb.exe in the next command.
dotnet run --project src/libraries/dotnet/RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release -- target/debug/rystemdb

RYSTEMDB_BINARY="$(pwd)/target/debug/rystemdb" \
  bash -c 'cd src/libraries/typescript/rystemdb-client && npm test'
```

Both languages test multiple outstanding operations, i64 minimum, byte arrays with `00 ff 80`, native references, nested arrays, non-ASCII document IDs (including U+FEFF), tombstones, missing records, invalid keys and restart persistence. The CI `.NET` and `npm/TypeScript` jobs **each run on Linux, macOS and Windows**, compile the actual native Rust server on that same runner, and run the live client subprocess tests there. All these SDK tests also verify that a second native CLI process cannot bypass the server's exclusive storage writer lock. Windows CI explicitly launches `target/debug/rystemdb.exe`; no fake process or port reservation is used. Rust native server/storage tests run across the same three operating systems. **Client packages are not published and cross-OS CI must not be mistaken for installed package smoke tests.**

See [RSWP framing](WIRE_PROTOCOL.md), [CRUD key spec](CRUD_KEY_PROTOCOL.md), [native RDB type spec](RDB_TYPED_PROTOCOL.md) and [security feature #39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

### CI and distribution boundaries

[CI workflow](../.github/workflows/ci.yml) has separate `ubuntu-latest`, `macos-latest`, and `windows-latest` matrices for **Node 22** and **.NET 10**. All test real binary CRUD and cross-process writer locking with the executable built locally on the runner. [Issue #48](https://github.com/KeyserDSoze/RystemDatabase/issues/48) tracks this specific acceptance gate; [#27](https://github.com/KeyserDSoze/RystemDatabase/issues/27) and [#28](https://github.com/KeyserDSoze/RystemDatabase/issues/28) still track actual NuGet/npm SDK packaging and authenticated remote connectivity. Existing published `v0.1.2` contains no new RPC server; no SDK package was released by these CI changes.

## Authenticated localhost mode (source-only, opt-in)

`rystemdb rpc-serve --auth-file ./private/acl.txt` requires a private-file 32-byte token with explicit `r`, `w` or `rw` permissions per collection; it denies anonymous HELLO and unauthorized operations with status `6`. The Rust, .NET and Node SDKs now expose `LocalClient::connect_with_token(addr, &token)`, `RystemLoopbackClient.ConnectWithTokenAsync(IPAddress.Loopback, port, token)` and `RystemLocalClient.connect("127.0.0.1", port, token)`. The token is **raw bytes**, not its hex representation. See the [exact file format, generation and code examples](LOCAL_AUTH.md), and TDD #49/#50. This is **not TLS**, cannot be bound externally and is not a substitute for #39.

## Rust-only verified TLS 1.3 mode (source preview)

Use `rpc-serve --tls-cert ... --tls-key ... --auth-file ...` with `LocalClient::connect_tls(addr, "localhost", ca_path, &token)`. This is an **explicit TLS-only listener**, not automatic TLS negotiation or a plaintext fallback. The Rust client verifies CA chain and SAN name before transmitting the token. Other SDKs **cannot yet connect to TLS**, and every listener still rejects external/nonloopback binds. See [LOCAL_TLS.md](LOCAL_TLS.md), scoped #51 and parent #39.
