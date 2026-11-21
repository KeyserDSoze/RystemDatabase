# ADR 0010 — Strict cross-language RDB v1 typed binary value codec

Status: accepted as experimental binary value format / SDK codec — 2026-11-21.

Related [#43](https://github.com/KeyserDSoze/RystemDatabase/issues/43), [#38](https://github.com/KeyserDSoze/RystemDatabase/issues/38), [#44](https://github.com/KeyserDSoze/RystemDatabase/issues/44).

## Decision

Implement native, dependency-light RDB binary value encoders/decoders in Rust, C#/.NET 10 and TypeScript/Node 22+, matching the Rust `rystemdb-core` v1 tag language byte-for-byte. Root types may be scalars, arrays or objects in this low-level value codec; **RPC PUT separately requires an object**.

C# uses a typed record union `RdbValue` with `long`, `double`, `byte[]`, arrays, dictionaries and native references. TypeScript uses `bigint` for signed i64, `number` for f64, `Uint8Array`, `Map`, `RdbReference`, arrays and null; this avoids JS precision loss and plain-object prototype collisions.

Sort all object fields **lexicographically by UTF-8 bytes**, not C# UTF-16 string order or JavaScript insertion order. Keep absence different from explicit null. Validate the same finite numeric, size, depth, truncation, UTF-8 validity, duplicate key and reference constraints as the Rust reference implementation. Inputs remain bounded to 64 MiB and max nesting 64. The remote RSWP limit is separately 16 MiB.

Use explicit UTF-8 encoders with strict surrogate behavior and decoders that preserve the leading U+FEFF scalar (Node's default BOM removal is incompatible with Rust/.NET). Never insert JSON serialization into the binary path.

## Verification

Independent literal golden vector for a nested typed object with native bytes, array, signed integer and reference. Negative tests cover truncated input at every boundary, duplicate keys, trailing bytes, invalid type tags, malformed UTF-8, invalid reference, depth exhaustion and non-finite floats; remaining cross-language *live socket* tests are a separate acceptance gate on #43, not implied by codec tests.

## Limits

C#/TS libraries are still **codecs only**, not actual network clients. EF Core provider, secure remote transport and cluster-aware reads are not implemented. This is not a claim of production readiness or high-performance benchmarks without measurements.

[Wire specification](../docs/RDB_TYPED_PROTOCOL.md).
