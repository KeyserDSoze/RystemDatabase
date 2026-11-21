# ADR 0007 — Loopback-only RSWP TCP vertical slice

Status: Accepted as experimental local development capability — 2026-11-21

Related issues: [#25](https://github.com/KeyserDSoze/RystemDatabase/issues/25), [#37](https://github.com/KeyserDSoze/RystemDatabase/issues/37), [#38](https://github.com/KeyserDSoze/RystemDatabase/issues/38), [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

## Context

We have Rust/C#/TypeScript RSWP binary framing codecs, but no real server. A local end-to-end RPC implementation is needed to validate framing, storage ownership and protocol behavior before adding remote security or distributed state.

## Decision

- Add a separate `src/rystemdb-cli/src/rpc.rs` module, exposed only as `rystemdb rpc-serve --data PATH [--listen LOOPBACK]`. The existing health-only HTTP `serve` is unchanged.
- Parse numeric `SocketAddr` and **reject non-loopback before opening any database directory**. Never expose plaintext unauthenticated document CRUD remotely.
- One shared `DocumentStore` is opened/locked at process startup. Bounded eight-worker pool + capacity-32 sync channel + per-socket 3s timeout + 128 frames/connection. A mutex serializes storage access and prevents accidental per-client lock duplication.
- Require a v1.0 HELLO byte payload before any RPC method; PING requires an empty body.
- Accept PUT/GET/DELETE using compact u16 BE collection and ID lengths, followed (PUT only) by a self-describing `rystemdb-core` binary object. GET returns the same binary document, not JSON.
- Strict bad-message rejection and bounded allocations: max 16MiB RSWP frame; nested values validated by the existing `RDB\0` codec. Successful PUT/DELETE responses occur only after the existing storage append + sync path reports success.
- Errors return status only, no filesystem path, segment detail or data.

## TDD evidence

RED contract [`415cedb`](https://github.com/KeyserDSoze/RystemDatabase/commit/415cedb06814263ff48d31c56e41b9debd40fcf9), [CI #44925604197](https://github.com/KeyserDSoze/RystemDatabase/actions/runs/44925604197) failed 5 missing-server behavioral tests on Linux and macOS. GREEN implementation [`75f609d`](https://github.com/KeyserDSoze/RystemDatabase/commit/75f609d77d67f4a76b900df8f89a4381866db6b2). Strict lint adjustment [`1d716f0`](https://github.com/KeyserDSoze/RystemDatabase/commit/1d716f0c40af9030b7f2a8a38ffdb885f9adb90e). Existing CLI test race in negative malformed-input paths required focused regression fix [`aef04f9`](https://github.com/KeyserDSoze/RystemDatabase/commit/aef04f98c48a0e7e61228545f8cd1a80dc959628) (#40).

Final documentation-inclusive CI and release runs are recorded on Issue #37.

## Non-goals

- Does **not** provide TLS/auth/authz, remote trusted transport, an async client SDK, distributed shard routing, replication, SQL, MVCC, full protocol negotiation or a durable query plan.
- The per-connection deadline is not a global operation deadline. DoS resistance, fair scheduling and resource controls require follow-ups.
- The CRUD body contract still needs shared library encoding helpers and independent C#/TypeScript golden conformance; see #38.
- Real power-loss durability, compaction and transactional indexes remain open under #2/#17.

This is a local single-node research/prototype milestone, not a production-ready network service.
