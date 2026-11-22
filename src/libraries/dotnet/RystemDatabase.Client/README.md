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

**Important limitations:** this is a source-only localhost TCP client without production remote authentication, audited mTLS identity lifecycle, pooling, multiplexing, query API, idempotent retry guarantees, shard routing, NuGet package publication or EF Core provider. Invalid/outside-loopback address rejected before connect; network failure on a write can mean an uncertain commit and is never automatically retried. Parent [#27](https://github.com/KeyserDSoze/RystemDatabase/issues/27) remains OPEN; scoped live-connection [#45](https://github.com/KeyserDSoze/RystemDatabase/issues/45).

[Local usage](../../../../docs/LOCAL_CLIENTS.md) | [Typed value format](../../../../docs/RDB_TYPED_PROTOCOL.md)

## Opt-in authenticated localhost HELLO

`ConnectWithTokenAsync(IPAddress.Loopback, port, byte[] token, CancellationToken)` accepts a raw **32-byte** token, sends authenticated HELLO and exposes status `6` for wrong credentials/forbidden collection operations via `RystemRpcException`. The Rust server must use `--auth-file` with an explicit per-collection `r/w` ACL. This is plaintext localhost development auth, not TLS or a published NuGet client. See [LOCAL_AUTH.md](../../../../docs/LOCAL_AUTH.md).


## Verified TLS 1.3 mode (localhost only)

```csharp
using System.Net;
using RystemDatabase.Client;
var token = await File.ReadAllBytesAsync("private/write.token");
await using var db = await RystemLoopbackClient.ConnectTlsAsync(
    IPAddress.Loopback, 8790, "localhost", "private/root-ca.pem", token);
await db.PingAsync();
```

`ConnectTlsAsync` uses `SslStream`, an explicit `X509ChainPolicy.CustomRootTrust`, standard DNS SAN verification (no validation bypass) and requires negotiated **TLS1.3** before sending the bearer HELLO. On macOS .NET 10, enable Network.framework using `DOTNET_SYSTEM_NET_SECURITY_USENETWORKFRAMEWORK=1` at startup; the SDK also opts in before making its TLS connection. The real Rust server subprocess, negative wrong CA/name/expired/not-yet-valid cert tests and binary CRUD are CI-tested on Linux/macOS/Windows (#52/#53/#54). This is **not** remotely deployable or published to NuGet; source-only mTLS client certificate transport is now separately available via `ConnectMtlsAsync` (#69), but certificate revocation/rotation, Windows key-container ACL review, pooling and cluster routing remain open. [LOCAL_TLS.md](../../../../docs/LOCAL_TLS.md).

## Opt-in real TLS1.3 client-certificate mTLS (#69, loopback source only)

```csharp
using System.Net;
using RystemDatabase.Client;
var token = await File.ReadAllBytesAsync("private/write.token");
await using var client = await RystemLoopbackClient.ConnectMtlsAsync(
    IPAddress.Loopback, 8790, "localhost",
    "private/server-root-ca.pem",
    "private/client-writer.pem", "private/client-writer.key", token);
await client.PingAsync();
```

Unlike `ConnectTlsAsync` (server certificate only), this method presents
a caller-supplied X.509 client certificate with a matching local private
PEM key through native `SslStream`. The Rust server must require
`--mtls-ca`, `--mtls-policy` and an authenticated HELLO token, then
intersects the strict verified client URI SAN principal ACL with the token
ACL. Local certificate/key files must be bounded (64 KiB each), regular,
non-symlink and, for private PEM on Unix, group/other inaccessible.
Mismatched cert/key is rejected *before connecting*.

On Windows, Schannel cannot use the PEM loader's purely in-memory private
key. The SDK uses a **temporary OS-managed user key import** via an
in-memory PKCS#12 roundtrip, zeroes the buffer and disposes the certificate.
**Windows may still briefly materialize a private key under the OS user
profile; NTFS ACL/cleanup and production key management are not audited.**
MacOS .NET 10 needs Network.framework TLS1.3 as noted above. Actual
Rust-process tests use ephemeral CA-signed writer and reader certificates
and verify positive typed CRUD and wrong issuer/EKU/identity/token,
read-only ACL and server CA/SAN on all three OSes.

This remains local experimental source, not a released/production remote
mTLS SDK. See [LOCAL_TLS.md](../../../../docs/LOCAL_TLS.md) and
[#55](https://github.com/KeyserDSoze/RystemDatabase/issues/55).
