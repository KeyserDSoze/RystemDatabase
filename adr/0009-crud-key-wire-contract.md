# ADR 0009 — Shared RSWP CRUD key-prefix format and cross-language strictness

Status: Accepted for **v1 key-prefix byte layout only**, 2026-11-21.

Parent [#38](https://github.com/KeyserDSoze/RystemDatabase/issues/38); scoped [#42](https://github.com/KeyserDSoze/RystemDatabase/issues/42); follow-up typed RDB [#43](https://github.com/KeyserDSoze/RystemDatabase/issues/43).

## Decision

- The CRUD key prefix has fixed u16 BE byte lengths for collection and ID, strict ASCII collection grammar, UTF-8 ID with no Cc controls, combined max 1024 bytes, and an optional opaque binary RDB suffix only for PUT.
- Shared Rust implementation in `rystemdb-protocol::crud` is used by both local TCP server and Rust `LocalClient`; no duplicate key validation/encoding logic.
- C# uses a strict UTF8Encoding (throw on decoding/encoding invalid sequences). JavaScript rejects unpaired UTF-16 surrogates and malformed UTF-8; neither silently replaces invalid identifiers.
- Rust decode borrows from the existing bounded RSWP payload, while C#/TS decode into owned values. All reject invalid/truncated lengths and over-limit data before accepting.
- Independently authored byte-exact vectors in Rust, C# and TS test `users/u42`, `orders/🦊`, and an opaque typed RDB body for `{"n":42}`. Negative tests cover invalid names, oversized keys and invalid UTF-8.
- This codec does not decode the document suffix or provide a remote client. Full typed RDB v1 interoperability is a separate prerequisite (#43). Only the RSWP v1 wire envelope and key prefix are now common across all three.

## Risk boundaries

- This is a preliminary format with no authenticated remote exposure. Nested binary typed values have no C#/TS decoder yet, therefore end-to-end interoperability in those languages is unproven. Maintain #38/#43 OPEN.
- Local TCP is still localhost-only until TLS1.3/authz #39, and a complete server/client behavior suite #25/#26–28.
- Do not claim this is the fastest format without reproducible comparisons.

## Verification

Scoped Issue #42 records the RED/GREEN commits and the final successful CI. [Byte-level contract](../docs/CRUD_KEY_PROTOCOL.md).
