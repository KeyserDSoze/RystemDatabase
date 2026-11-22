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

**Strict decode rule (#65):** every object's key sequence MUST be strictly
increasing by the raw UTF-8 byte comparison, recursively at every nesting
depth. Decoders in Rust, .NET and TypeScript now reject decreasing distinct
keys as **noncanonical**, rather than accepting several byte encodings of one
logical object. Duplicate keys remain rejected with their separate existing
duplicate-key errors. Compare **UTF-8 bytes**, not UTF-16 order or Unicode
locale collation: a BMP U+E000 key (`ee 80 80`) sorts **before** the astral
U+1F600 key (`f0 9f 98 80`). Encoding continues to sort fields; this rule
does not change the wire envelope or any tag/version numbers.

**Compatibility boundary:** earlier experimental source-only decoders accepted
out-of-order *distinct* keys even though encoders always produced sorted keys
and this protocol document prescribed canonical ordering. Those previously
accepted **noncanonical input bytes now fail closed**. Existing conforming
RDB v1 documents remain byte-identical. Do not silently recanonicalize
untrusted invalid bytes or deploy a new public format without a migration /
version-negotiation policy under #43/#38.

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
- Maximum **262,144 aggregate tagged value nodes per document** (root value **included**; each array, object, byte/string, scalar or typed reference is one node, and every nested array/object element value consumes another). Object **key names** and string bytes are not separate tagged value nodes; their bytes count against the encoded byte limit. This applies equally to **encoding and decoding** in Rust, .NET and Node. Container declared lengths are checked against the *remaining shared node budget before allocating collection capacity or iterating children*. A tiny dense/null frame can otherwise amplify into many heap values.
- Maximum recursive nesting depth: **64** (root depth 0).
- Reject truncated length prefixes and bodies, malformed UTF-8, unsupported format version/tags, **duplicate or out-of-order object keys**, unknown/trailing bytes, non-finite floats, invalid references and excessive sizes before allocating unreasonable buffers.
- JavaScript and .NET writers reject isolated invalid UTF-16 surrogate sequences rather than substituting a Unicode replacement character.
- **Leading `U+FEFF` in a UTF-8 string is data**, not a removable file BOM; TypeScript readers deliberately set `TextDecoder(..., {fatal:true, ignoreBOM:true})` to preserve it, confirmed by TDD regression #44.
- The primitive tagged format is compatible with the experimental binary row codec; schema field IDs, decimal128, compression, lazy field projection and versioned migrations remain future engine work (#33).

## Aggregate decoded value-node budget (#71, cross-language hardening)

The 64 MiB input frame limit alone did not cap heap amplification: an
experimental **262,154-byte** RDB array containing 262,144 null values was
previously accepted by every decoder, even though it creates **262,145
tagged value nodes** including the root array. Rust/Node/.NET RED tests
reproduced that behavior. At most **262,144** nodes may now be created
from one RDB frame; a root array with exactly **262,143 null children**
fits. Nested arrays/objects share one budget across the entire frame:
a root of 512 arrays each containing 512 nulls is over budget even
though no one immediate array contains more than 512 entries.

On decode the budget is checked before creating collections
(in particular, before the .NET array's eager `List<RdbValue>` capacity
allocation) and incremented for every recursively parsed tagged value.
Encoders enforce the identical budget so they do not produce new bytes
the decoders cannot accept. Existing canonical, in-budget RDB v1 frames,
including the golden vector and typed CRUD, remain byte-identical.

**Compatibility:** this adds a **deliberately stricter acceptance rule**
to the unreleased experimental RDB v1 codec. Previously accepted
overbudget typed documents are now refused with an RDB size/node-limit
error; no input is truncated or silently repaired. Do not deploy this
without a version/migration policy for stored nonconforming documents.
This is **not** a measured performance improvement, a maximum heap-size
guarantee (long strings, dictionaries, language object overhead), full
fuzz coverage, request rate limiting or a production DoS threat model.
These remain open under #43/#11/#39.

## Next end-to-end gates

The native Rust `LocalClient` and loopback `rpc-serve` communicate using this typed codec. The .NET/TS libraries now **encode/decode** the native bytes **and connect over localhost TCP** to the actual Rust `rpc-serve` process with HELLO/PING/PUT/GET/DELETE and persistence checks ([LOCAL_CLIENTS.md](LOCAL_CLIENTS.md), #45/#46). They still require authenticated TLS networking, production resources, cross-platform client package smoke and full version-negotiated protocol tests (#39/#43/#27/#28) before any external server.

See [RSWP frame](WIRE_PROTOCOL.md), [CRUD key prefix](CRUD_KEY_PROTOCOL.md), [local RPC server](RPC_SERVER.md) and [ADR 0010](../adr/0010-typed-rdb-cross-language.md).
