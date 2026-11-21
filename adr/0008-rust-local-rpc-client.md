# ADR 0008 — Initial synchronous Rust localhost RPC client

Status: experimental developer-only API — 2026-11-21

Related: [#26](https://github.com/KeyserDSoze/RystemDatabase/issues/26), [#37](https://github.com/KeyserDSoze/RystemDatabase/issues/37), [#38](https://github.com/KeyserDSoze/RystemDatabase/issues/38), [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

## Context

The server's RSWP frame and local binary CRUD protocol need a real Rust client API before beginning async pooling and cross-language SDK ports.

## Decision

- Add `LocalClient` to `src/libraries/rust/rystemdb-client`, with a single persistent synchronous `TcpStream`, 3s read/write/connect timeout, TCP_NODELAY and numeric `SocketAddr`.
- Reject any non-loopback IP **before TCP connect**; remote connectivity must wait for TLS1.3/authz.
- Perform mandatory HELLO v1.0 handshake at connection creation, then `ping`, typed binary document `put/get/delete` on the same connection with monotonic 64-bit request IDs and exact method/response matching.
- Encode the provisional compact key schema from the RPC server's documentation, with strict collection, ID and body size checks. Data values use the existing `RDB\0` typed codec rather than JSON.
- Return explicit error categories; only status=1 maps to missing document / false delete. Write/disconnect timeouts are **uncertain**, not automatic safe retries.
- Test against a **real server subprocess**, not a mock socket, on the CI OS matrix. Validate remote IP prohibition and typed nested bytes/references.

## Pending

- Avoid duplicate request-body key implementations by moving versioned helpers into `rystemdb-protocol` and adding independent C#/TS golden CRUD vectors (#38).
- Async nonblocking client, multiplexing/pooling, deadlines/cancellation, idempotency, TLS/auth, shard topology and real release packaging (#26/#39).
- No claims that this constitutes a complete remote database SDK or production-ready system.

Exact GREEN CI and commit links are recorded in #26. Keep #26 OPEN until the full SDK's acceptance criteria pass.
