# Experimental storage format v1 (source on main)

This is not yet a standalone database service. Only the **embedded Rust crate** `rystemdb-storage` implements durable append-only document frames; the `rystemdb` CLI and prebuilt `v0.1.1` releases still expose a health-only server.

## Layout

```text
data/
  segment-00000001.rdb
  segment-00000002.rdb
  ...
```

One file contains a sequence of variable-length frames, each storing one PUT of a typed nested binary document or a DELETE tombstone. Frame structure (network byte order for lengths and CRC):

```text
magic[4] = "RYSF"
format_version: u8 = 1
operation: u8 = 1 (PUT) or 2 (DELETE)
reserved: u16 = 0
id_length: u32
value_length: u32
crc32: u32
id: [u8; id_length] // UTF-8, 1..1024 bytes
value: [u8; value_length] // rystemdb-core codec v1, or empty for DELETE
```

The checksum is CRC-32/ISO-HDLC over bytes [4..16] followed by ID and payload; it excludes magic and the checksum field. Values are binary typed documents, not JSON text. Different or absent nested fields remain supported because the codec is self-describing.

## Write, read and recovery contract

1. Validate ID and serialize the value completely before writing.
2. Append one complete frame. Sync file data before publishing the updated in-memory ID map.
3. Get by ID locates file+offset and verifies checksum and value. Delete appends a tombstone.
4. On restart, replay every segment, creating the ID index from the latest PUT/DELETE for each ID. Last segment only: truncate a physically incomplete final frame. Complete corrupt frames and older truncated frames are **rejected**, not skipped.
5. Sync failures make the current writer unusable until reopen; a failed write may have an uncertain outcome after a crash.

Read/write/delete/rollover/restart, malformed input, corruption and truncated-tail behavior are covered by Cargo integration tests.

## Experimental immutable committed read views (#79; child of #6)

The embedded Rust crate exposes `DocumentStore::snapshot()`, returning a
`DocumentSnapshot` with `get(id)`, `len()` and `is_empty()`. Snapshot
capture clones an **owned live ID → segment/offset map**, a
**last-committed ID → segment/offset generation ledger** (including tombstones)
and database path, not document payload bytes. After #81 this is
**O(number of distinct IDs ever written)** metadata/time per capture,
not merely the number of live documents.
A snapshot reflects the set of acknowledged committed versions at capture
time. Subsequent PUT/DELETE/new inserts only affect the live store index and
append new RYSF frames; earlier versions remain readable by their original
offsets, including across normal segment rotation and writer close/reopen.
Each snapshot `get` opens the historical on-disk frame, verifies its CRC32,
record ID and RDB codec, and returns **the original version**, never a stale
unchecked in-memory value or the writer's newer version. Read-only snapshots
do **not** acquire or hold the exclusive OS writer lock, and can be moved to a
reader thread while a cooperative writer appends. The current design has no
compaction and does not remove old committed frames.

These views are **in-process experimental metadata**, not durable snapshot
identifiers or multi-document transactions. External filesystem corruption,
uncoordinated recovery, deleting old segments or any future compaction without
reader-aware version retention can invalidate a view. There are **no**
concurrent multi-writer transactions, multi-document write-write conflict detection,
serializable isolation, phantom protection, WAL/commit epochs, MVCC GC, or
replication. The parent [#6](https://github.com/KeyserDSoze/RystemDatabase/issues/6)
remains OPEN. Regression: `cargo test -p rystemdb-storage --locked --test snapshot_contract`.

## Existing-key optimistic conditional writes (#80; child of #6)

`DocumentStore::replace_if_unchanged(&snapshot, id, &value)` and
`delete_if_unchanged(&snapshot, id)` provide **single-document optimistic
concurrency checks** using the committed `(segment, offset)` recorded in
`DocumentSnapshot`. The snapshot must refer to the same storage root/path,
and the document must have **existed at capture time**. A changed, deleted
or replaced version returns structured `StoreError::Conflict` without any
append or index mutation; even an overwrite with identical value bytes has a
new location and causes a conflict. A snapshot from a different root returns
`StoreError::InvalidSnapshot`. An unchanged key can be updated or tombstoned
by the single exclusive writer. Unrelated keys may change without causing
conflicts. Old snapshot reads still return their original verified versions,
including across segment rollovers and writer close/reopen.

**The existing-key replacement/deletion methods still require a key that
was present in the snapshot**; absent-key inserts use the separate #81 API.
Foreign-root checks compare path metadata, **not** a durable cryptographic
database identity; path replacement, symlink aliases,
uncoordinated recovery/compaction or external disk edits are not handled
as transactional operations. No multi-key commit, serializable isolation,
persistent version tokens, compaction retention, or remotely distributed
concurrency control is implemented. Parent #6 remains open.

Regression: `cargo test -p rystemdb-storage --locked --test conditional_update`.

## ABA-safe absent-key conditional insert (#81; child of #6)

`DocumentStore::insert_if_unchanged(&snapshot, id, &value)` allows a
single-key INSERT only if **the key was absent in that snapshot, is still
absent now, and its last-committed generation matches**. The writer maintains
a second `ID → (segment,offset)` generation ledger for *every* ID seen
in an acknowledged RYSF PUT or DELETE frame, keeping DELETE tombstones
instead of discarding their offsets. It is rebuilt from existing binary
segments on restart and extended only after successful append/fsync.
Thus an initial absent snapshot is rejected if the key was inserted and
deleted in the meantime (**ABA**), even after rollover and reopen.
A snapshot captured after a tombstone can authorize an insert as long as
the generation is unchanged. Current `get` and `len` still use only
the live-documents index; old snapshot reads verify the historical CRC.

This is intentionally a **memory-unbounded ledger of distinct IDs ever
written**, plus O(distinct IDs) per captured snapshot; repeated generations
of the same ID replace one map entry, but churn of new IDs grows retained
metadata. No durable global epoch/DB identity, compaction protocol,
version reclamation, concurrent multi-writer transactions or multi-key
serializability is implemented. Future compaction must preserve this
generation knowledge to avoid false successful inserts.

Regression: `cargo test -p rystemdb-storage --locked --test conditional_insert`.

The **feature-enabled real-process fault matrix** covers pre-append, torn synced payload, complete pre-sync write, post-sync/pre-index publication (PUT and DELETE), and post-directory-sync segment rotation. It is run on Linux, macOS and Windows.

## Crash semantics (test-only)

`cargo test -p rystemdb-storage --locked --features fault-injection --tests -- --test-threads=1` intentionally starts subprocesses which exit with code `97` at controlled storage boundaries, skipping Rust destructors. Only this test build reads `RYSTEMDB_TEST_CRASH_POINT`. Never build a release with `fault-injection` enabled. See [ADR 0003](../adr/0003-process-crash-fault-injection.md) and [crash test matrix](../tests/CRASH_GATE.md).

- Before append: existing acknowledged data survives; new value absent.
- Mid-frame with a persisted partial payload: only the incomplete tail of the **last** segment is repaired.
- Full write **before** sync: the new value may or may not replay, but must not be partial or invalid.
- After successful sync **before** in-memory publication: a complete PUT or DELETE must replay.
- After new segment creation and directory sync: prior documents survive and new empty segment is reusable.

These tests simulate **process termination**, not disk failure or sudden power loss.

## Simulated filesystem operation failures (issue #20)

In addition to abrupt process exits, `src/rystemdb-storage/tests/io_failure.rs` runs fault-enabled **child processes that stay alive after receiving a synthetic `StoreError::Io`**. A failed append/sync/segment-creation puts that open store into a permanently **poisoned** state; every later mutation, including a no-op DELETE, returns `StoreError::Poisoned`. The store must be closed and reopened to rebuild a verified index from binary frames.

- `io_before_append`: reject before writing; previously acknowledged data preserved.
- `io_short_write`: persist a deliberately incomplete frame, return an injected I/O error, then truncate only the final incomplete tail during replay.
- `io_sync_data`: simulate a failed data flush **before** the real flush; the full unsynced record may or may not be observed after recovery, but is never treated as partially valid.
- `io_after_sync`: report an error after the successful flush but before index publication. Recovery must replay the fully durable PUT or DELETE even though the caller received an uncertain error.
- `io_segment_create`: report an error before creating a rollover segment.
- `io_segment_sync`: report an error after creating the new empty segment but before its file synchronization.
- `io_segment_directory_sync`: report an error after syncing the new segment but before syncing its directory entry.

The fault points **simulate the error return without forcing the underlying filesystem or hardware to genuinely fail**. They are compiled out of normal binaries, tested explicitly by the no-feature isolation test, and must never be enabled in shipped releases. This does **not** prove actual power-loss durability, Windows directory-entry behavior, or atomic multi-record transactions. See [ADR 0004](../adr/0004-io-failure-poisoning.md) and [test gate matrix](../tests/CRASH_GATE.md).

## Writer-lock contention/replay regression evidence (#19/#64)

The storage uses a **permanent** `rystemdb.writer.lock` inode and Rust
`File::try_lock`. A small `WriterLockGuard` now explicitly unlocks in its
destructor on both a normal `DocumentStore` close **and every early error**
during opening/replay (#66). The lock guard is established immediately after
the OS lock succeeds and is the store's last field. Lock ownership is
advisory and OS-dependent. The original single Linux-only flaky
test on 2026-11-21 did **not** print the underlying reopen error; subsequent
tests report `StoreError` explicitly and cover 50 sequential same-process
contenders, 40 immediate synced-write reopenings and 12 independent-process
contention/replay cycles.

Additional test-only characterizations (`#64`) exercise 16 rounds where
**eight threads simultaneously** attempt to open the same store while the
single writer holds the advisory lock: every contender must get
`StoreError::Locked`. The owner then drops and a fresh writer immediately
reopens and verifies its synchronized sentinel, **without retry**.
A second test runs 24 iterations where a deliberately corrupted, previously
synced final frame makes `DocumentStore::open` fail *after acquiring the
lock*. Restoring only the test fixture's byte immediately permits a new
writer to replay the acknowledged value. No corruption is ignored in the
storage implementation.

The [macOS CI run #45059826197](https://github.com/KeyserDSoze/RystemDatabase/actions/runs/45059826197)
produced **actual error `Locked`** on the *immediate reopen* after an
intentionally corrupt replay, despite the failed open returning
`StoreError::Corrupt`. Prior to #66, `DocumentStore::drop` explicitly unlocked
only successful opens; early `?` exits during replay left the acquired lock
to implicit file-handle close. The RAII guard fixes that error path without
modifying the advisory locking algorithm, weakening corruption rejection,
removing the lockfile or adding retries. The older isolated Linux incident
has not been independently proven to have the same cause.

The Linux CI Rust job independently repeats the isolated lock regressions
**six times** and fails on any iteration; the macOS CI Rust job now repeats
the early-Err corrupt replay → immediate reopen regression **ten independent
times** to guard against the observed intermittent `Locked` result (#66).
Neither job retries an individual failed open, ignores a failed test, or uses
`continue-on-error`. The ordinary Rust and fault-injection matrix continues
on all three OSes. These tests improve confidence but do
**not prove the root cause** of the original intermittent Linux failure or
guarantee lock correctness for all filesystems/adversarial processes.
Parent [#19](https://github.com/KeyserDSoze/RystemDatabase/issues/19)
remains open for root-cause investigation.

## Explicit limitations

- **Advisory single-writer exclusion:** `rystemdb.writer.lock` is held with `File::try_lock` on the entire store lifetime, and released by the OS after normal or abrupt process exit. Two cooperative Rystem writers cannot open the same directory concurrently. Uncooperative tools or filesystems without reliable locks can still damage data.
- Rebuild-from-log on every startup (not yet an index engine).
- Only experimental in-process read views and single-key optimistic conditional PUT/DELETE/INSERT with retained tombstone generations; **no transactional MVCC engine**, SQL, analytics projections, foreign-key enforcement, compaction, backup/restore or transactions spanning documents.
- Operating-system and hardware flush semantics may differ. Power-loss tests and Windows directory durability remain necessary.
- CRC32 is not a cryptographic integrity guarantee; use trusted local storage.
- No guarantee for hostile filesystem modifications or arbitrary bit rot, beyond fail-closed checks for recognized corruption.

See [ADR 0002](../adr/0002-append-only-document-segments.md) and [Issue #2](https://github.com/KeyserDSoze/RystemDatabase/issues/2) for remaining acceptance gates.
