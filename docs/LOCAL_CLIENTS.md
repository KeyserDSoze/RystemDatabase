# Rystem Database — experimental local client connections

This page describes the **source-only localhost TCP client milestone**, developed directly on main under [#45](https://github.com/KeyserDSoze/RystemDatabase/issues/45), [#46](https://github.com/KeyserDSoze/RystemDatabase/issues/46), and parent [#27](https://github.com/KeyserDSoze/RystemDatabase/issues/27)/[#28](https://github.com/KeyserDSoze/RystemDatabase/issues/28).

**Security warning:** the Rust RSWP RPC server does **not implement TLS, authentication, authorization, or public network policy**. It will only bind numeric loopback. All three SDKs reject non-loopback connection targets. Do not use port-forwarding or public reverse proxies to bypass that restriction. A production remote client requires [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

**Release warning:** the current published native release \`v0.1.2\` predates this RPC server and these SDK connection implementations. Build \`rystemdb\` from main to run this example. None of the .NET/TypeScript client packages is published to NuGet/npm at this point; npm/cli is an independent native-binary launcher.

## Run a real Rust server

\`\`\`sh
cargo build --locked -p rystemdb-cli
./target/debug/rystemdb rpc-serve --data ./rystem-data --listen 127.0.0.1:8790
\`\`\`

The server owns the exclusive binary segment writer lock for its entire process lifetime; you cannot concurrently open a second local CLI writer on that directory.

## C#/.NET 10 (asynchronous, one persistent socket)

Use the project in \`src/libraries/dotnet/RystemDatabase.Client\`, or add it as a project reference to your own solution.

\`\`\`csharp
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
\`\`\`

\`ConnectAsync\` validates the numeric IP before socket creation and performs RSWP 1.0 HELLO. Async operations take optional \`CancellationToken\`, use 3-second per-call deadlines, validate the 24-byte frame and response ID/opcode, and serialize I/O through a semaphore. Cancellation or broken TCP cannot guarantee the outcome of a write; the connection closes rather than blindly replaying it. A missing GET becomes \`null\`, and a second DELETE yields \`false\`. Errors return explicit exceptions / \`RystemRpcException.Status\`.

**Not yet implemented**: TLS/auth, pooling, multiplexing, automatic safe idempotency/retries, query execution, shard-aware placement and NuGet distribution. Parent #27 remains open.

## TypeScript / Node.js 22+ (asynchronous, correlated concurrent requests)

Use \`src/libraries/typescript/rystemdb-client/src/client.ts\` alongside the existing \`wire.ts\`, \`crud.ts\`, \`rdb.ts\` from this source tree.

\`\`\`typescript
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
\`\`\`

\`connect\` permits only numeric \`127.0.0.1\` and \`::1\` (no DNS). A native socket \`Buffer\` is normalized to **plain \`Uint8Array\`**, with independent golden tests plus a real-process regression for a Buffer subclass bug [#47](https://github.com/KeyserDSoze/RystemDatabase/issues/47). The socket parser handles TCP frame fragmentation, guards 16 MiB per message, and correlates concurrent requests by native BigInt u64 IDs. A bounded pending-request map and 3-second inactivity/request timers fail closed on lost connection. No automatic write retries or full streaming queries.

**Not yet implemented**: TLS, authentication, user-provided AbortSignal, connection pools/retry policy, browser support, NuGet/npm SDK release, sharding or geo-redundancy. Parent #28 remains open.

## Real Rust-process CI verification

These are **not mock tests**: each live test launches the Rust CLI as an actual OS subprocess, uses \`--listen 127.0.0.1:0\` to atomically select a port inside the server, parses the flushed startup line, exercises HELLO/PING/native nested binary PUT/GET/DELETE and a true process restart, then closes all processes.

\`\`\`sh
cargo build --locked -p rystemdb-cli
dotnet run --project src/libraries/dotnet/RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release -- target/debug/rystemdb

RYSTEMDB_BINARY="$(pwd)/target/debug/rystemdb" \
  bash -c 'cd src/libraries/typescript/rystemdb-client && npm test'
\`\`\`

Both languages test multiple outstanding operations, i64 minimum, byte arrays with \`00 ff 80\`, native references, nested arrays, non-ASCII document IDs (including U+FEFF), tombstones, missing records, invalid keys and restart persistence. The CI \`dotnet\` and \`npm\` jobs now compile the actual native Rust server binary and run these live subprocess tests on Linux. Independent Rust native server/storage tests already run across Linux/Windows/macOS. Cross-OS *client-language* CI and release packaging are still future work.

See [RSWP framing](WIRE_PROTOCOL.md), [CRUD key spec](CRUD_KEY_PROTOCOL.md), [native RDB type spec](RDB_TYPED_PROTOCOL.md) and [security feature #39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).
