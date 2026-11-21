# ADR 0003 — Feature-gated process crash injection

Status: Accepted — 2026-11-21
Related: [#18](https://github.com/KeyserDSoze/RystemDatabase/issues/18), #17, #2

## Context
The append-only binary segment store syncs a frame before publishing it in the in-memory ID map, but normal single-process tests cannot prove its restart behavior when the process stops between steps.

## Decision
- Add an opt-in `fault-injection` feature to `rystemdb-storage`. Disabled by default.
- Under this feature only, `RYSTEMDB_TEST_CRASH_POINT` selects a hook in `try_append`: before append, after segment rotation/directory sync, during a partially persisted frame, after writing a full frame before sync, and after sync before returning to the caller.
- Terminate the child via `std::process::exit(97)` to skip Rust destructors and close OS resources, then reopen the store in the parent process.
- Expose **no** environment-triggered termination in ordinary Rust production builds. Never enable this feature in distributed release packages.
- Add subprocess integration tests on all three CI operating systems, including a synchronized DELETE tombstone.
- For a pre-sync crash, accept either absence or a complete verified record. For a post-sync crash, require the full record/tombstone to replay. Fail closed for corrupt complete frames.
- Keep `#2` / `#17` OPEN: these tests do not demonstrate physical power-loss persistence, index transaction atomicity, or compaction safety.

## Evidence
- Genuine [RED commit `19f9a70`](https://github.com/KeyserDSoze/RystemDatabase/commit/19f9a7088b65b21404ae2c0e08dcaad19efda1de), [CI 44915918271](https://github.com/KeyserDSoze/RystemDatabase/actions/runs/44915918271): six expected missing-hook failures.
- Implementation [`f3d6208`](https://github.com/KeyserDSoze/RystemDatabase/commit/f3d6208a39c1262d96f54c3e2589765778f6e52a).
- Full CI [44916120711](https://github.com/KeyserDSoze/RystemDatabase/actions/runs/44916120711) passed after an additional lock reopen stress/diagnostic test (see #19). Final docs-inclusive commit verification is recorded in #18.
