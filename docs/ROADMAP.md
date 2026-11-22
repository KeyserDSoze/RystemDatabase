# Roadmap — Rystem Database

All work is tracked by GitHub Issues, developed directly on main, with RED → GREEN → REFACTOR and the main CI gates.

## Foundation (0.1.x, **not** a usable database)
- [x] Rust workspace, nested binary codec, CLI health server
- [x] GitHub Actions pipeline definition and release packaging configuration
- [x] npm/npx launcher source, Linux/macOS/Windows installers, container definitions
- [x] Confirm a successful CI matrix and publish first signed-off preview artifacts (v0.1.1)
- [ ] Reserve npm package name and configure NPM_TOKEN before npm publishing
- [x] End-to-end installer + CRUD smoke against published v0.1.2 binaries on Linux, macOS and Windows (#21/#22/#36)

## Storage (0.2.x)
- [x] Experimental self-describing v1 on-disk framing, checksums and append-only segments (schema dictionary still pending)
- [x] Rebuildable in-memory document ID → segment/offset lookup with restart replay, plus incomplete final tail truncation
- [x] Cooperative cross-process single-writer lock with tests for abrupt process exit
- [x] Test-only deterministic process-crash injection at six append/sync/rollover boundaries (#18)
- [x] Synthetic filesystem I/O error returns with poisoned-writer replay contracts (#20)
- [ ] Physical power-loss testing, fault-tolerant durable index publication, compaction and recovery (#2, #17)
- [x] Embedded source API: put/get/delete (overwrite via append) typed nested values; no network API yet
- [x] Native CLI document put/get/delete over binary store with stdin/file JSON and typed reference/byte tags (#21)
- [x] Verified native v0.1.2 document CRUD prerelease, container persistence and installer smoke across three OS (#21/#22/#36); npm/npx publication still pending (#1)

## Relations and query (0.3.x)
- [ ] Equality, range, composite and array element indexes
- [ ] Referential constraints and reverse edges with transactional updates
- [ ] DataFusion integration, table provider, planner, joins, filter pushdown

## Analytics (0.4.x)
- [ ] Parquet projections, Arrow batches, manifest and change overlay
- [ ] MVCC/snapshot consistency tests and compaction policy
- [ ] Benchmarks versus PostgreSQL JSONB and DuckDB

## Production readiness (not date-committed)
- [ ] Auth and transport security, backup/restore, upgrade compatibility
- [ ] Fuzz/property tests and Windows/macOS/Linux crash/recovery matrix
- [ ] Profiling, p95/p99 metrics, memory/space benchmarks, operations runbooks

## Issue status and closure policy

GitHub Issues are the source of truth, not static checkboxes alone. [Browse open issues](https://github.com/KeyserDSoze/RystemDatabase/issues?q=is%3Aissue+is%3Aopen). Each change is committed directly to `main`, attached to the owning issue with RED/implementation/final-GREEN SHAs. Close only when the scoped issue's acceptance criteria and exact final `main` CI are satisfied. Do not close parent issues because a narrower child issue was completed. In particular, #1 remains open for real installer/npx validation; #2 and #17 for storage durability/compaction; #19 for investigation of a lock reopen flake.

## Network and SDKs (planned)

- [x] Create four `src/libraries` project folders: Rust, .NET, TypeScript and EF Core (#26–29)
- [x] RSWP v1 fixed binary framing and Rust/.NET/TypeScript golden vector test infrastructure (#24/#27/#28, partial milestones)
- [x] RSWP key-prefix v1 shared Rust codec in server/client, independent C#/TypeScript UTF-8 golden/negative tests (#42)
- [x] Independent Rust/.NET/TypeScript native RDB value codec with nested arrays/bytes/references, i64, null and golden/error conformance (#43, partial)
- [x] Source-only .NET async and Node.js TypeScript correlated TCP HELLO/typed PUT/GET/DELETE against real Rust server subprocess with restart, Unicode, concurrency tests (#45/#46)
- [x] Live .NET 10 / Node.js 22 Rust RPC interoperability CI on Linux, macOS and Windows, including competing writer rejection, no preallocated ports and Windows .exe path (#48)
- [x] Optional local-only authenticated HELLO + deny-by-default collection `r/w` ACL from private credential file; real Rust/.NET/Node SDK tests across all three OS (#49/#50)
- [x] Experimental Rustls TLS 1.3-only loopback listener + Rust client certificate-chain/SAN verification, no plaintext fallback, real negative TLS tests (#51)
- [x] Real TLS1.3 CA+SAN-verified local .NET 10 SslStream and Node 22 node:tls clients with typed CRUD and negative security tests on Linux/macOS/Windows (#52); macOS .NET Network.framework backend (#53)
- [x] Flush native Rustls RSWP auth-denial replies before closing TLS sessions (#54)
- [x] Canonical bounded principal policy and fail-closed validation (#57); opt-in Rustls client-certificate verification gate (no RSWP authorization yet, #58)
- [x] Explicit TLS1.3-only Rust client certificate presentation with bounded protected key files and ephemeral Rustls interop tests (#60); **real mTLS CLI still denies RSWP**
- [ ] Extract verified URI SAN principal and authorize via intersected certificate/token ACLs, then end-to-end Rust/.NET/Node mTLS (#56/#55)
- [ ] Production TLS/mTLS, external identity, token expiry/rotation/revocation, Windows ACL audit, per-user quotas and rate limiting (#39)
- [ ] Full version-negotiated v1 body compatibility, fuzz/property coverage, cancellation/idempotency policy and multi-OS client SDK release tests (#43/#38)
- [x] Experimental loopback-only RSWP native TCP binary CRUD server with bounded workers and TDD subprocess tests (#37)
- [x] Synchronous Rust `LocalClient` proof-of-concept: real HELLO/PING/typed CRUD and loopback enforcement (#26, partial)
- [ ] Rust SDK async pooling and .NET/TypeScript secure transport, cancellation/retry, package publishing, shard routing and full multi-OS support (#26–28, #39)
- [ ] Authenticated native RPC server, fully interoperable wire CRUD body schema, session pooling and complete SDK connections (#25–28, #38, #39)
- [ ] Genuine EF Core provider semantics, LINQ translation, tracking and EF specification suite (#29)
- [ ] Production TLS/cert validation, authn/authz, flow control, idempotency, version negotiation (#24/#25)
- [ ] Arrow Flight SQL streaming analytical endpoint and interoperability (#35)

## Cluster and storage performance (planned)

- [ ] Deterministic shard placement/epoch, online rebalancing and scatter-gather queries (#30)
- [ ] Replicated WAL with quorum leadership, fencing, failover and restore (#31)
- [ ] Geo-redundancy with RPO/RTO and regional consistency policy (#32)
- [ ] Sparse typed binary v2 with field IDs/varints/compression and migration benchmarks (#33)
- [ ] Adaptive indexes and vectorized query benchmarks, no unverified performance claims (#34)
