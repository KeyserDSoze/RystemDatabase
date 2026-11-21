# RystemDatabase.Client (.NET 10)

[C# SDK Issue #27](https://github.com/KeyserDSoze/RystemDatabase/issues/27).

**Implemented:** standalone `RystemWireCodec` with RSWP v1 24-byte header, big-endian IDs/lengths, binary payloads, input-size bounds and request/response status checks. The `RystemDatabase.Client.Tests` independent executable verifies the Rust golden PING vector, 64-bit IDs and truncated frames.

**Not implemented:** an operational remote `RystemDbClient` connection, TCP/TLS/auth, request multiplexing, typed CRUD bodies, cluster discovery and NuGet publishing. These require server #25 and full conformance.

Build and conformance:

```sh
dotnet run --project ../RystemDatabase.Client.Tests/RystemDatabase.Client.Tests.csproj --configuration Release
```

The EF Core integration is a separate project (#29), not part of this wire codec.
