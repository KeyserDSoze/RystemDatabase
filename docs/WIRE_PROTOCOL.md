# Rystem Wire Protocol — RSWP v1 (experimental)

Source of truth: [Issue #24](https://github.com/KeyserDSoze/RystemDatabase/issues/24), `src/rystemdb-protocol`. A versioned, **binary frame** carries the request/response metadata and an opaque binary payload. It is conceptually similar to JSON-RPC (method + request ID + response status), but JSON is **not used** for the frame envelope or its planned document payload.

**Status:** the frame codecs are tested in Rust, C#/.NET and TypeScript, and an experimental **loopback-only** Rust TCP server now implements a provisional binary document CRUD body. This is **not** an authenticated remote server or cross-language CRUD client; there is no TLS, authz or distributed routing. No version with the RPC server has been released yet.

## Exact header v1

All integers are big-endian (network byte order). Each frame is exactly `24 + payload_length` bytes:

| Offset | Bytes | Field | Value / meaning |
|---:|---:|---|---|
| 0 | 4 | magic | ASCII `RSWP` = `52 53 57 50` |
| 4 | 1 | major | `1` |
| 5 | 1 | minor | `0` |
| 6 | 1 | kind | `1` request, `2` response |
| 7 | 1 | operation | `1` HELLO, `2` PING, `16` PUT, `17` GET, `18` DELETE |
| 8 | 2 | status | request = `0`; response = `0..7` |
| 10 | 2 | header_length | `24` (v1 fixed) |
| 12 | 8 | request_id | `u64`, for pairing responses with requests |
| 20 | 4 | payload_length | `u32`, 0..16 MiB |
| 24 | N | payload | opaque binary data, format is operation-specific |

Status: `0` success, `1` not found, `2` invalid request, `3` busy, `4` internal, `5` unsupported version, `6` unauthorized, `7` redirect. Unknown kinds, operations, statuses, header sizes or major/minor version are rejected by v1 decoders. Request status must be zero. Requests/responses are self-delimiting, can be concatenated on persistent streams, and reject extra bytes when decoding a single complete frame.

### Byte-exact golden vector

For `PING`, request ID `7`, no payload:

```text
52 53 57 50 01 00 01 02 00 00 00 18
00 00 00 00 00 00 00 07 00 00 00 00
```

Rust, C# and TypeScript independent conformance tests assert these octets. All implementations must reject truncation and oversized payload length **before allocation**.

## Provisional document payload v1 (local server on main)

A **working local-only** Rust server uses binary HELLO/PING and PUT/GET/DELETE: [exact body layout, statuses and safety limits](RPC_SERVER.md). Request key: `u16 collection_len BE` + ASCII collection + `u16 id_len BE` + UTF-8 ID, plus one `RDB\\0` typed document payload for PUT. GET responds with binary `RDB\\0` value, never JSON. The body schema remains **experimental** until shared SDK helpers and independently validated C#/TypeScript CRUD golden vectors exist (#38); frame conformance alone does not establish full client interoperability.

Long queries and data streams need bounded multi-frame chunking, cancellation, deadlines, flow control and pagination; large analytics results are better served as Arrow IPC/Flight SQL when DataFusion is implemented. Client request ID correlation and server response status are already represented by the v1 header, but idempotency tokens and topology/shard epochs are a later negotiated schema.

## Transport, safety and performance

Candidate transport for point queries: **persistent TCP** with **TLS 1.3 + authenticated clients** before any non-loopback availability. Multiplexing and pooling must be implemented and benchmarked; there is no universally fastest protocol. Compare RSWP/TCP against gRPC/Protobuf and QUIC with the same payloads, p50/p95/p99 latency, bytes/request, CPU, memory, loss and WAN RTT. Avoid unmeasured superiority claims.

This frame does not include its own MAC/CRC: authenticated TLS provides integrity in the remote transport; the storage format has a separate CRC to detect accidental file corruption. Do not confuse checksum validation with secure authentication.

Data safety requirements before remote activation: length bounds, no memory amplification, protocol version negotiation, TLS/cert validation, auth, replay/idempotency policy, cancellation/deadlines, flow control, resource isolation, secure defaults, fuzz testing, and secret-safe error messages.

## Distribution and sharding considerations

The distributed extension must include a topology map, shard placement epoch, leader term, retry/redirect contract, read consistency levels, commit high-watermark and idempotency operation ID. No request should be retried blindly across an uncertain write. Define per-shard consensus and geo-replication separately before enabling distributed writes.

See [distributed architecture](DISTRIBUTED_ARCHITECTURE.md), [ADR 0006](../adr/0006-binary-rpc-clients-and-distribution.md), [Issue #30](https://github.com/KeyserDSoze/RystemDatabase/issues/30), [#31](https://github.com/KeyserDSoze/RystemDatabase/issues/31), and [#32](https://github.com/KeyserDSoze/RystemDatabase/issues/32).
