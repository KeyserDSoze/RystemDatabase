# Typed RDB binary document v1 — Rust / .NET / TypeScript

Canonical Rust format in `src/rystemdb-core/src/lib.rs`. A **self-describing binary value codec** (not a JSON serializer, table provider or transactional database). Independent native codecs now exist in .NET 10 and Node.js/TypeScript, with byte-level golden and negative tests.

Status: **codec layer implemented and tested in three languages on `main`**, but **no live .NET or TypeScript TCP database clients** yet. Full end-to-end interop remains tracked in [#43](https://github.com/KeyserDSoze/RystemDatabase/issues/43), parent [#38](https://github.com/KeyserDSoze/RystemDatabase/issues/38). The last published `v0.1.2` is the local document CLI only.

## Encoding

Every value begins with ASCII `RDB\0` (hex `52 44 42 00`) and format version `01`. A recursive value follows:

| Tag | Type | Subsequent bytes (all lengths and fixed numbers big-endian) |
|---|---|---|
| `00` | Null | None |
| `01` | Boolean false | None |
| `02` | Boolean true | None |
| `03` | Signed integer | 8-byte two's-complement i64 |
| `04` | Floating point | 8-byte IEEE 754 f64 (finite only) |
| `05` | UTF-8 string | u32 byte length + raw bytes |
| `06` | Raw bytes | u32 byte length + raw bytes |
| `07` | Array | u32 item count + recursive values |
| `08` | Object | u32 field count + (u32 UTF-8 field-name length + bytes + recursive value) repeated |
| `09` | Typed reference | u32 UTF-8 collection length + bytes + u32 UTF-8 ID length + bytes; both nonempty |

Object fields are encoded in **UTF-8 byte lexicographic order**, matching Rust `BTreeMap<String,Value>`. In .NET, native ordinal UTF-16 sorting gives a different order for some supplementary characters; the codec sorts by UTF-8 bytes. The TypeScript codec does the same. An explicit `null` field (tag `00`) is distinct from a completely absent field.

### Language mappings

| RDB type | Rust | C#/.NET | TypeScript |
|---|---|---|---|
| i64 | `Value::Integer(i64)` | `RdbValue.Integer(long)` | `bigint` |
| f64 | `Value::Float(f64)` | `RdbValue.Float(double)` | finite `number` |
| bytes | `Value::Bytes(Vec<u8>)` | `RdbValue.Bytes(byte[])` | `Uint8Array` |
| reference | `Value::Reference` | `RdbValue.Reference` | `RdbReference` |
| object | `Value::Object(BTreeMap)` | `RdbValue.Object(IReadOnlyDictionary)` | `Map<string,RdbValue>` |
| array | `Value::Array(Vec)` | `RdbValue.Array(IReadOnlyList)` | `RdbValue[]` |
| explicit null | `Value::Null` | `new RdbValue.Null()` | `null` |

Never silently convert an i64 to a JavaScript `number` (precision is lost above 2^53). TypeScript **requires `bigint`** for RDB integer values; a JS `number` intentionally becomes f64. Monetary decimal128 is not yet supported; f64 is not exact decimal money.

### Nested golden vector

For an object with fields in encoded order:

```text
a = [null, true, -1]
b = bytes(00 ff 80)
r = ref("users", "u42")
```

The exact 66-byte RDB value is:

```text
52444200010800000003
00000001610700000003000203ffffffffffffffff
0000000162060000000300ff80
00000001720900000005757365727300000003753432
```

The literal is asserted independently in `src/rystemdb-core/tests/rdb_compat.rs`, `src/libraries/dotnet/RystemDatabase.Client.Tests/Program.cs`, and `src/libraries/typescript/rystemdb-client/tests/rdb.test.mjs`.

## Safety and format limits

- Maximum complete encoded value: **64 MiB**. The RPC envelope separately limits a single request/response payload to **16 MiB**, so not every locally valid RDB value can be sent in one RPC frame.
- Maximum recursive nesting depth: **64** (root depth 0).
- Reject truncated length prefixes and bodies, malformed UTF-8, unsupported format version/tags, duplicate object keys, unknown/trailing bytes, non-finite floats, invalid references and excessive sizes before allocating unreasonable buffers.
- JavaScript and .NET writers reject isolated invalid UTF-16 surrogate sequences rather than substituting a Unicode replacement character.
- **Leading `U+FEFF` in a UTF-8 string is data**, not a removable file BOM; TypeScript readers deliberately set `TextDecoder(..., {fatal:true, ignoreBOM:true})` to preserve it, confirmed by TDD regression #44.
- The primitive tagged format is compatible with the experimental binary row codec; schema field IDs, decimal128, compression, lazy field projection and versioned migrations remain future engine work (#33).

## Next end-to-end gates

The native Rust `LocalClient` and loopback `rpc-serve` communicate using this typed codec. The .NET/TS libraries now **encode and decode** the native bytes independently, but still need real socket clients and live PUT/GET/DELETE against the Rust process (#27/#28/#43), followed by authenticated TLS network transport (#39) before any external server.

See [RSWP frame](WIRE_PROTOCOL.md), [CRUD key prefix](CRUD_KEY_PROTOCOL.md), [local RPC server](RPC_SERVER.md) and [ADR 0010](../adr/0010-typed-rdb-cross-language.md).
