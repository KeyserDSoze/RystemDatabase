# Roadmap — Rystem Database

All work is tracked by GitHub Issues, developed directly on main, with RED → GREEN → REFACTOR and the main CI gates.

## Foundation (0.1.x, **not** a usable database)
- [x] Rust workspace, nested binary codec, CLI health server
- [x] GitHub Actions pipeline definition and release packaging configuration
- [x] npm/npx launcher source, Linux/macOS/Windows installers, container definitions
- [ ] Confirm a successful CI matrix and publish first signed-off preview artifacts
- [ ] Reserve npm package name and configure NPM_TOKEN before npm publishing
- [ ] End-to-end smoke tests against actual released binaries on every supported OS

## Storage (0.2.x)
- [ ] Stable on-disk framing, versioned schema IDs, checksums, append-only segments
- [ ] Durable document ID → segment/offset lookup and restart recovery
- [ ] Atomic indexing transactions, crash fault injection, compaction
- [ ] Document API: insert/get/update/delete with typed nested values

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
