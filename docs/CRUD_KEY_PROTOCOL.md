# RSWP v1 CRUD key contract — byte-for-byte cross-language

Source: [Issue #42](https://github.com/KeyserDSoze/RystemDatabase/issues/42), parent protocol [#38](https://github.com/KeyserDSoze/RystemDatabase/issues/38).

**Implemented in Rust, C#/.NET and TypeScript** in source on `main`, with independent golden/negative conformance tests. This describes the key prefix and opaque PUT bytes; the separate [RDB v1 value codec](RDB_TYPED_PROTOCOL.md) now has independent cross-language golden tests, while live socket interoperability remains tracked in [#43](https://github.com/KeyserDSoze/RystemDatabase/issues/43).

## Layout

The body for RSWP `GET`/`DELETE` consists only of:

| Order | Field | Encoding | Constraint |
|---|---|---|---|
| 1 | Collection length | u16 big-endian | 1–128 |
| 2 | Collection bytes | ASCII | `[A-Za-z0-9_-]` |
| 3 | ID length | u16 big-endian | At least 1 |
| 4 | Document ID | UTF-8 | Nonempty, no Unicode category Cc control characters |
| 5 | Document suffix | Empty for GET/DELETE | Any extra bytes are rejected by the server |

The size in **bytes** of `collection + "/" + id` is at most **1024**. Unicode identifiers are permitted; UTF-8 byte counts, *not* UTF-16 code units or Unicode character counts, govern the limits. Strict decoders reject malformed UTF-8, invalid names, zero-length IDs, oversized IDs, truncated lengths or data. TypeScript's encoder rejects isolated UTF-16 surrogates rather than silently replacing them. The Rust decoder returns borrowed slices; C# and TypeScript return the value and copy the optional document bytes.

`PUT` appends one complete typed `RDB\0` binary `Value::Object` after these fields. That binary suffix is preserved **as-is** by the key decoder; the Rust server separately validates it as a nested document object. Request frame payloads are capped at **16 MiB** by the RSWP envelope, and the key decoder rejects larger inputs independently. Do not conflate the 64 MiB embedded RDB document codec limit with the narrower network frame limit.

## Cross-language golden vectors

### GET key `users/u42`

```text
00 05   75 73 65 72 73   00 03   75 34 32
│         users            │         u42
collection length = 5    ID length = 3
```

### PUT key `orders/🦊`

```text
00 06   6f 72 64 65 72 73   00 04   f0 9f a6 8a
│              orders       │          fox emoji UTF-8
collection length = 6      ID byte length = 4
```

For a PUT containing exactly `{"n":42}` as a typed binary RDB v1 object, append these opaque bytes directly to the key above:

```text
52 44 42 00   01   08   00 00 00 01   00 00 00 01   6e   03   00 00 00 00 00 00 00 2a
  RDB magic    ver   map    field count    name length     n   i64          signed value 42
```

These literal vectors are tested in all three languages. This format **does not send JSON text**, and it does not serialize missing fields as explicit nulls.

## Shared Rust implementation

`src/rystemdb-protocol/src/crud.rs` is the canonical codec for both the native TCP server and `LocalClient`. The original duplicated parsers in `src/rystemdb-cli/src/rpc.rs` and `src/libraries/rust/rystemdb-client/src/lib.rs` now delegate to it. Its decoder validates bounds and returns borrowed `&str` and `&[u8]` values, avoiding intermediate JSON, UTF-8 buffer copies or unnecessary allocations.

Independent language implementations:
- `src/libraries/dotnet/RystemDatabase.Client/RystemCrudKey.cs` — `RystemCrudKey.Encode/Decode`.
- `src/libraries/typescript/rystemdb-client/src/crud.ts` — `encodeCrudKey/decodeCrudKey`.
- `src/rystemdb-protocol/tests/crud_contract.rs` — Rust golden/error tests.
- `src/libraries/dotnet/RystemDatabase.Client.Tests/Program.cs` — C# golden/error tests.
- `src/libraries/typescript/rystemdb-client/tests/crud.test.mjs` — TypeScript golden/error tests.

No public remote server, authentication/TLS, async Rust connection, full C#/TS network SDK or EF provider is implemented by this codec. Cross-language typed binary values are now covered separately in [RDB_TYPED_PROTOCOL.md](RDB_TYPED_PROTOCOL.md). The server remains **loopback-only**. See [RPC_SERVER.md](RPC_SERVER.md), [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39), [#43](https://github.com/KeyserDSoze/RystemDatabase/issues/43) and [#25](https://github.com/KeyserDSoze/RystemDatabase/issues/25).
