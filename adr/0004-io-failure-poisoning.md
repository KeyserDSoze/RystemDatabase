# ADR 0004 — Synthetic I/O errors and the poisoned-writer state

Status: Accepted (experimental single-writer store) — 2026-11-21

Related: [#20](https://github.com/KeyserDSoze/RystemDatabase/issues/20), #17, #2, #19

## Problem

A process can receive an error from an append, a file sync, or a rollover operation while some data has already reached the OS. If it continues writing with an uncertain file tail or stale in-memory index, it could misrepresent commit outcomes or corrupt subsequent frames.

## Decision

- An error in the append/write/rotation/sync path marks the `DocumentStore` instance as poisoned. Any later PUT or DELETE, including an absent-document no-op DELETE, must fail as `StoreError::Poisoned`, even if its input is invalid. Read operations on already indexed data are still possible but cannot be treated as resolving an uncertain mutation.
- Reopen the store to reconstruct the in-memory ID→segment/offset index from fully validated frames; truncate only an incomplete tail of the last segment.
- A fully written frame whose sync reported an error has an **uncertain outcome**: it may or may not be visible after replay. Return success only when the write and sync path completed normally.
- A synthetic error after sync but before index publication must replay the full PUT or DELETE after restart.
- Under the explicit test-only Cargo feature `fault-injection`, `RYSTEMDB_TEST_IO_ERROR` selects a deterministic injected `StoreError::Io` at append, short write, data sync and segment rollover/sync boundaries. The short-write hook persists an incomplete tail to exercise recovery.
- Fault hooks are compiled out when the feature is disabled; normal builds ignore the environment variable. Never ship binaries with `fault-injection` enabled.

## Limitations

- Synthetic errors model return values, not real kernel write failures, filesystem bugs or power removal.
- Power-loss and Windows directory durability remain open in #17/#2.
- Multi-document ACID, secondary indexes, compaction and proven serializable writes remain unimplemented.
- CRC32 detects accidental corruption, not hostile tampering.

## TDD evidence

- RED: [commit `2e07ed5`](https://github.com/KeyserDSoze/RystemDatabase/commit/2e07ed578818341b6850df4252bd1bbb461dafda); [release verify #44920345176](https://github.com/KeyserDSoze/RystemDatabase/actions/runs/44920345176) failed in 9 I/O scenarios.
- GREEN implementation: [commit `f190f11`](https://github.com/KeyserDSoze/RystemDatabase/commit/f190f1180d37038e0472cc25567d90a978496c05).
- Complete final CI evidence will be recorded in Issue #20.
