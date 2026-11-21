# Rystem Database client libraries

Four project directories, all developed directly on `main`:

| Library | Path | Stage | Issue |
| --- | --- | --- | --- |
| Rust | `rust/rystemdb-client` | Experimental **synchronous** `LocalClient` with HELLO/PING/typed CRUD over a real **loopback TCP** server, tested; no async/pool/TLS | #26 |
| .NET | `dotnet/RystemDatabase.Client` | .NET 10 RSWP framing codec tested; no live connection yet | #27 |
| TypeScript | `typescript/rystemdb-client` | Node.js 22+ TypeScript RSWP framing codec tested; no live connection yet | #28 |
| EF Core | `dotnet/RystemDatabase.EntityFrameworkCore` | Buildable .NET project scaffold only, **not an EF Core provider** | #29 |

The separate `src/rystemdb-protocol` crate is the binary wire frame reference (#24); an experimental loopback-only server is now being implemented under #37, but TLS/auth (#39), **working client SDK connections** and shared interoperable CRUD payload schemas (#38) remain open work.

Conformance source/tests:
- Rust: `src/rystemdb-protocol/tests/wire_contract.rs`
- C#: `src/libraries/dotnet/RystemDatabase.Client.Tests/Program.cs`, run under the .NET 10 SDK
- TypeScript: `src/libraries/typescript/rystemdb-client/tests/wire.test.mjs`, Node 22+

The three frame codecs share a literal octet-level PING vector and reject unsupported versions/oversized or truncated frames. They also now implement independent [CRUD key-prefix golden vectors](../../docs/CRUD_KEY_PROTOCOL.md) (#42); the complete RDB typed document suffix is not yet implemented by .NET/TypeScript (#43). The Rust SDK now includes a **real localhost-only synchronous connection**; the .NET/TypeScript SDKs remain framing codecs, not operational clients. Rust async pooling, authenticated TLS, cancellation and shard-aware routing are tracked in #26/#39.

See [wire protocol](../../docs/WIRE_PROTOCOL.md), [ADR 0006](../../adr/0006-binary-rpc-clients-and-distribution.md) and [distributed design](../../docs/DISTRIBUTED_ARCHITECTURE.md). Do not publish client packages as operational until server+SDK end-to-end tests pass.
