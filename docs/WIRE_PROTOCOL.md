# Rystem Wire Protocol — RSWP v1 (experimental)

Source of truth: [Issue #24](https://github.com/KeyserDSoze/RystemDatabase/issues/24), `src/rystemdb-protocol`. A versioned, **binary frame** carries the request/response metadata and an opaque binary payload. It is conceptually similar to JSON-RPC (method + request ID + response status), but JSON is **not used** for the frame envelope or its planned document payload.

**Status:** RSWP frames, CRUD key prefixes and typed RDB document bodies have independent Rust/.NET/TypeScript byte-conformance tests, and all three SDKs now connect to an experimental **loopback-only** Rust TCP server across Linux/macOS/Windows. A **local-only, opt-in authenticated HELLO with collection ACLs** exists ([LOCAL_AUTH.md](LOCAL_AUTH.md)), but this is **not TLS or remotely secure**. No SDK or native RPC release has been published.

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

### Server pre-authentication frame budget (#72)

The RSWP v1 **wire format is unchanged**. Rust's
`rystemdb_protocol::read_frame_with_limit(reader, max_payload)` reuses
the exact v1 header validator and rejects any declared body length
exceeding the caller-specified maximum **before allocating or reading
body bytes**; it also enforces the normal global 16 MiB maximum.
`read_frame(reader)` continues to allow 16 MiB on standard frames.

The source-only Rust `rpc-serve` selects `max_payload=36` until its
first valid HELLO/authentication succeeds: the 2-byte version HELLO and
36-byte token HELLO still work, but a 37-byte header declaration is
rejected without waiting for an absent body. Once greeted, it uses the
original 16 MiB standard reader for typed CRUD. This is server-side
framing control, **not** a protocol version change or a reduction of the
authenticated CRUD payload allowance. Broader slowloris protections,
per-identity quotas, TLS handshake resource controls and fuzz tests are
still required before any external bind under #39.

## Provisional document payload v1 (local server on main)

A **working local-only** Rust server uses binary HELLO/PING and PUT/GET/DELETE: [exact body layout, statuses and safety limits](RPC_SERVER.md). Request key: `u16 collection_len BE` + ASCII collection + `u16 id_len BE` + UTF-8 ID, plus one `RDB\\0` typed document payload for PUT. GET responds with binary `RDB\\0` value, never JSON. The CRUD key-prefix schema now has a **shared Rust decoder** and independent .NET/TypeScript golden vectors ([CRUD_KEY_PROTOCOL.md](CRUD_KEY_PROTOCOL.md), #42). The complete **typed RDB binary document suffix** and live Rust/.NET/TypeScript localhost client connections now have end-to-end test coverage, including [optional local-token HELLO and collection-specific authorization](LOCAL_AUTH.md). Stable negotiation, authenticated TLS remote transport, streaming and published SDK compatibility remain open (#38, #39, #43).

Long queries and data streams need bounded multi-frame chunking, cancellation, deadlines, flow control and pagination; large analytics results are better served as Arrow IPC/Flight SQL when DataFusion is implemented. Client request ID correlation and server response status are already represented by the v1 header, but idempotency tokens and topology/shard epochs are a later negotiated schema.

## Optional local-only authenticated HELLO

By default (legacy localhost development), HELLO request payload is `01 00`. When the Rust server starts with `--auth-file`, HELLO must instead contain `01 00 00 20` followed by exactly **32 raw bearer-token bytes**. Successful HELLO responds with `01 00`. Missing/incorrect credentials receive status `6` and the connection closes; authenticated sessions receive per-collection `r`/ `w` grants, and unauthorized GET/PUT/DELETE return status `6` without accessing the store. Plaintext localhost remains available for the older clients; an **opt-in verified TLS 1.3 transport between the Rust server and Rust/.NET/Node SDKs** now exists ([LOCAL_TLS.md](LOCAL_TLS.md)). This is still **not remote deployment security**, and full mTLS/security and Windows ACL gates remain open (#39). See [LOCAL_AUTH.md](LOCAL_AUTH.md).

## Transport, safety and performance

Experimental point queries now use **persistent localhost TCP**; Rust/.NET/Node SDKs can opt into verified **TLS 1.3 with bearer-token ACL** ([LOCAL_TLS.md](LOCAL_TLS.md)). **Non-loopback remains prohibited.** Multiplexing and pooling must be implemented and benchmarked; there is no universally fastest protocol. Compare RSWP/TCP against gRPC/Protobuf and QUIC with the same payloads, p50/p95/p99 latency, bytes/request, CPU, memory, loss and WAN RTT. Avoid unmeasured superiority claims.

This frame does not include its own MAC/CRC: authenticated TLS provides integrity in the remote transport; the storage format has a separate CRC to detect accidental file corruption. Do not confuse checksum validation with secure authentication.

Data safety requirements before remote activation: length bounds, no memory amplification, protocol version negotiation, TLS/cert validation, auth, replay/idempotency policy, cancellation/deadlines, flow control, resource isolation, secure defaults, fuzz testing, and secret-safe error messages.

## Distribution and sharding considerations

The distributed extension must include a topology map, shard placement epoch, leader term, retry/redirect contract, read consistency levels, commit high-watermark and idempotency operation ID. No request should be retried blindly across an uncertain write. Define per-shard consensus and geo-replication separately before enabling distributed writes.

See [distributed architecture](DISTRIBUTED_ARCHITECTURE.md), [ADR 0006](../adr/0006-binary-rpc-clients-and-distribution.md), [Issue #30](https://github.com/KeyserDSoze/RystemDatabase/issues/30), [#31](https://github.com/KeyserDSoze/RystemDatabase/issues/31), and [#32](https://github.com/KeyserDSoze/RystemDatabase/issues/32).
