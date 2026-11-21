# ADR 0011 — Real .NET/Node client TCP connections to Rust, localhost only

Status: accepted as a developer-only integration milestone — 2026-11-21

Issues: #45 (.NET), #46 (Node), #47 (Buffer mismatch), parents #27/#28/#43, security #39.

## Context

After the independent binary RSWP framing, CRUD key and RDB typed codecs matched Rust/.NET/TypeScript golden bytes, they still lacked live .NET and TypeScript TCP connections. Stubs did not prove an SDK worked across processes.

## Decision

- Add an **asynchronous .NET 10** \`RystemLoopbackClient\` with one persistent \`TcpClient\` and a semaphore to serialize requests and safely correlate responses. Strict 16MiB frame bound before allocation, 3-second per-call deadlines, optional cancellation tokens, typed RDB CRUD and \`IAsyncDisposable\`.
- Add **Node.js 22+** \`RystemLocalClient\` using \`node:net\`, a bounded receive buffer with framed incremental extraction and a pending-request map keyed by BigInt u64 IDs; concurrent requests correlate individually. Enforce per-request timeout and at most 120 pending operations.
- Validate numeric loopback-only targets at API entry. TLS 1.3/authn/authz are **not** implemented; no external address, no DNS or public port forwarding is permitted until #39.
- Reuse three existing strict codec layers without adding a lossy JSON serialization hop. Preserve U+FEFF and signed i64 exactness and explicitly distinguish nullable missing document from explicit typed null.
- Do **not** retry uncertain writes after TCP failure. Client errors/protocol mismatches fail closed.
- In CI, build the **actual native Rust executable** in both npm and .NET jobs, spawn independent Rust OS processes with \`rpc-serve --listen 127.0.0.1:0\`, parse its actual reported port to avoid TOCTOU, verify HELLO/PING/typed PUT/GET/DELETE, concurrency, a refused competing native writer and replay across true process restart. The .NET/Node tests are now executed on Linux, macOS and Windows (#48).
- Canonicalize incoming Node socket Buffer subclasses to the promised Uint8Array SDK type; a real-process test caught and scoped this regression (#47).

## Not decided / follow-ups

TLS authentication and remote deployment (#39), SDK package publishing/version matrix (#27/#28), cross-OS live SDK e2e (completed for Linux/macOS/Windows in #48; **published client SDK package installation is still pending**), async Rust pooling (#26), optimistic concurrency, idempotency tokens, full version negotiation, query planning, sharding/replication and EF Core provider (#29).

This does not make Rystem a secure production multi-client database. Current release v0.1.2 includes only local document CLI; new RPC/SDK behavior is on main source only.

[Client docs](../docs/LOCAL_CLIENTS.md) | [Wire spec](../docs/WIRE_PROTOCOL.md).
