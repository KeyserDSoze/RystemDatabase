# RystemDatabase.Client (.NET 10, experimental localhost-only SDK)

Features completed on source main:
- `RystemWireCodec`, `RystemCrudKey`, `RystemRdbCodec`: exact RSWP v1 binary framing, collection/ID keys and nested typed RDB document values, matching Rust and TypeScript byte-for-byte.
- **`RystemLoopbackClient`**: async single persistent TCP connection to the real Rust `rpc-serve` process; RSWP v1 HELLO, PING, typed PUT/GET/DELETE, 3-second deadline, cancellation, bounded response payload, response correlation and status checks.
- `IAsyncDisposable`, serialized asynchronous calls with a semaphore, no JSON intermediates, distinct missing/DELETE semantics.
- CI on Linux/macOS/Windows builds `rystemdb` in Rust and executes genuine .NET subprocess interoperability tests with nested typed values, UTF-8 IDs, same-connection concurrent operations, deletion, and actual server restart.

```sh
cargo build --locked -p rystemdb-cli
dotnet run --project src/libraries/dotnet/RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release -- target/debug/rystemdb
```

Example:

```csharp
using RystemDatabase.Client;
using System.Net;
await using var db = await RystemLoopbackClient.ConnectAsync(IPAddress.Loopback, 8790);
await db.PingAsync();
var value = new RdbValue.Object(new Dictionary<string,RdbValue> {
    ["name"] = new RdbValue.Text("Ada")
});
await db.PutAsync("users", "u42", value);
var found = await db.GetAsync("users", "u42");
```

**Important limitations:** this is a source-only localhost TCP client without TLS/secure remote authentication, pooling, multiplexing, query API, idempotent retry guarantees, shard routing, NuGet package publication or EF Core provider. Invalid/outside-loopback address rejected before connect; network failure on a write can mean an uncertain commit and is never automatically retried. Parent [#27](https://github.com/KeyserDSoze/RystemDatabase/issues/27) remains OPEN; scoped live-connection [#45](https://github.com/KeyserDSoze/RystemDatabase/issues/45).

[Local usage](../../../../docs/LOCAL_CLIENTS.md) | [Typed value format](../../../../docs/RDB_TYPED_PROTOCOL.md)

## Opt-in authenticated localhost HELLO

`ConnectWithTokenAsync(IPAddress.Loopback, port, byte[] token, CancellationToken)` accepts a raw **32-byte** token, sends authenticated HELLO and exposes status `6` for wrong credentials/forbidden collection operations via `RystemRpcException`. The Rust server must use `--auth-file` with an explicit per-collection `r/w` ACL. This is plaintext localhost development auth, not TLS or a published NuGet client. See [LOCAL_AUTH.md](../../../../docs/LOCAL_AUTH.md).


**TLS compatibility note:** the native server now supports opt-in TLS 1.3 via `--tls-cert --tls-key --auth-file` for the **Rust client only**. This .NET `RystemLoopbackClient` still uses local plaintext TCP. It cannot connect to a TLS-only listener; independent verified `SslStream` support is pending under [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39). [LOCAL_TLS.md](../../../../docs/LOCAL_TLS.md).
