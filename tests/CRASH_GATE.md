# Crash and durability verification gates

## Implemented test matrix (issue #18)

The integration tests at `src/rystemdb-storage/tests/crash_injection.rs` launch a **fresh OS child process** from the Rust test executable. With `fault-injection` enabled, the child's storage code exits with status `97` at exactly the selected write boundary, without unwinding the stack. Both default and feature-enabled cargo tests run in CI on Linux, Windows and macOS.

| Point | What was reached | Recovery rule |
| --- | --- | --- |
| `before_append` | No bytes appended | Prior acknowledged record remains; new absent |
| `partial_frame` | Intentionally synced half-frame | Incomplete tail of last segment is truncated |
| `after_frame_before_sync` | Complete frame written, unsynced | Uncertain outcome, but no partial/corrupt value |
| `after_sync_before_publish` (PUT) | Frame synced, in-memory map not updated | PUT visible after log replay |
| `after_sync_before_publish` (DELETE) | Tombstone synced, in-memory map not updated | Deletion visible after replay |
| `after_rotate` | New segment created, synced directory on Unix | Old records retained, empty new segment reusable |

Run locally:

```bash
cargo test --workspace --locked --all-targets
cargo test -p rystemdb-storage --locked --features fault-injection --test crash_injection -- --test-threads=1
cargo clippy --workspace --locked --all-targets --all-features -- -D warnings
```

The fault hooks exist **only in binaries compiled with** `--features fault-injection`. A plain `cargo build --release` excludes the hooks. Never distribute a feature-enabled binary.

## Still unproven and tracked in #2 / #17 / #6

- Actual power interruption/hardware cache loss or filesystem remount across real platforms, including Windows directory-entry guarantees.
- Deterministic injected `write`, `sync_data`, `sync_all` and directory-sync **errors** with a poisoned writer and uncertain commit semantics.
- Crash-safe, multi-document transactions, atomic durable secondary indexes or foreign-key enforcement.
- Interrupted compaction, snapshot retention, backup/restore correctness, concurrent readers and adversarial corruption.
- Strong isolation levels beyond single-writer sequential in-memory ID lookup.

A successfully completed process-crash simulation is **not** evidence of physical power-loss durability. Keep the parent issues open.
