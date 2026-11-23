# Experimental storage format v1 (source on main)

This is not yet a standalone database service. Only the **embedded Rust crate** `rystemdb-storage` implements durable append-only document frames; the `rystemdb` CLI and prebuilt `v0.1.1` releases still expose a health-only server.

## Layout

```text
data/
  segment-00000001.rdb
  segment-00000002.rdb
  ...
```

One file contains a sequence of variable-length frames, each storing one PUT of a typed nested binary document, a DELETE tombstone, or (#82, source `main` only) an atomic BATCH of 2..=1,024 such operations. Frame structure (network byte order for lengths and CRC):

```text
magic[4] = "RYSF"
format_version: u8 = 1
operation: u8 = 1 (PUT), 2 (DELETE), or 3 (BATCH; #82)
reserved: u16 = 0
id_length: u32 // 1..1024 for PUT/DELETE; ZERO for BATCH
value_length: u32 // <=64 MiB
crc32: u32
id: [u8; id_length] // UTF-8, 1..1024 bytes for PUT/DELETE; absent for BATCH
value: [u8; value_length] // typed RDB, empty DELETE, or bounded BATCH body
```

The checksum is CRC-32/ISO-HDLC over bytes [4..16] followed by ID and payload; it excludes magic and the checksum field. Values are binary typed documents, not JSON text. Different or absent nested fields remain supported because the codec is self-describing.

**BATCH (#82) wire extension:** operation `3` reserves an empty outer ID
(`id_length=0`). The checksummed `value` is a **single bounded binary
batch body**: `count:u16 BE` (2..=1,024), followed by `count` members
`op:u8 (1 PUT or 2 DELETE), id_length:u16 BE, id:UTF-8 bytes,
value_length:u32 BE, value:RDB bytes` (DELETE must have zero value bytes).
Every ID is nonempty, validated with the same storage ID rules, unique within
the batch, and no trailing body bytes are permitted. **All** values are
decoded and checked, with one outer CRC, before replay publishes **any**
member. Total outer body <=64 MiB. At current head all batch member
IDs share the same `(segment, frame offset)` version, and an indexed read
rechecks the full frame checksum and decodes the matching document.

**Compatibility:** old `PUT=1`/`DELETE=2` bytes remain unchanged and new
source can replay legacy segments. Older `v0.1.2` binaries do **not**
understand `BATCH=3` and must fail closed on these new records; **do not
downgrade** a data directory written with `commit_batch`. A successful CI
native/container workflow is not a newly published versioned release.

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

## Atomic single-frame multi-key commits (#82; child of #6)

The embedded Rust crate now exposes
`DocumentStore::commit_batch(&snapshot, &[BatchMutation::Put{id,value},
BatchMutation::Delete{id}, ...])`. It checks *all affected IDs* against
the snapshot's last-committed version ledger (including tombstones), verifies
the key set is unique, requires DELETE to address a present document, and
serializes all 2..=1,024 typed operations **before writing any bytes**.
One checksummed BATCH record is appended and synced; only after successful
fsync does the writer publish the entire new live index and tombstone
generations. A conflicting/invalid/oversized batch returns an error with
**no append or partially visible update**. Batch records may cross segment
rollovers, while preexisting snapshots can still read old versions.

Recovery never applies half of a batch: an incomplete final BATCH frame is
truncated as a whole; a **complete, valid and synced** frame fully replays.
A crash after the last write but before sync may replay the *entire* frame
or none, as in the original single-operation durability contract. A complete
frame with damaged CRC or invalid member data fails closed. Feature-gated
real subprocess tests cover before append, torn tails, full pre-sync frames,
and after sync/before publication. These are *process crash simulations*, not
a proof of durability against sudden physical power loss.

This is **not** general transaction isolation, multi-frame commit, a query
transaction manager, multi-writer concurrency, distributed consensus, or
a production ACID/MVCC claim. It allocates a bounded batch frame in memory,
and can only group changes fitting in a single <=64 MiB payload. Old
binaries cannot read the extended BATCH opcode.

Regression:
`cargo test -p rystemdb-storage --locked --test batch_commit`
(and with `--features fault-injection` for process-crash tests).

## Automatically tracked point reads for optimistic batch commits (#84; child of #6)

`DocumentStore::tracked_snapshot()` returns an owned, read-only
`TrackedSnapshot` with `get(&mut self, id)` and `read_count()`. Every
**successful** point `get` automatically records that exact ID as a
read dependency, including reads returning `None`, without requiring
the caller to manually list it. Repeated reads of the same ID count once;
an explicit RDB `Null` is distinct from absence and all typed document
values retain the historical snapshot semantics.

```rust
let mut view = db.tracked_snapshot();
let bob = view.get("oncall/bob")?;
// Choose changes based on bob, then commit later, after other writes may
// have occurred; all successfully observed point IDs are checked.
db.commit_tracked_batch(&view, &[
    BatchMutation::Put { id: "oncall/alice", value: &Value::Bool(true) },
    BatchMutation::Put { id: "audit/decision", value: &Value::Bool(true) },
])?;
```

`commit_tracked_batch` reuses the exact `commit_batch_checked`
preconditions with the automatically retained read IDs. If another write
changes a previously read point key, even through
`absent → PUT → DELETE → absent` or a PUT with identical payload,
the batch fails with `StoreError::Conflict` **before appending data**.
An unobserved key changing does not create a conflict. The tracked view
owns no writer lock; historical point reads remain stable during rollover,
writer reopening, and concurrent cooperative appends. The underlying
single-frame synced BATCH3 encoding, maximum 1,024 mutation count, and
64 MiB frame limit do not change.

A tracked view holds at most **1,024 distinct successful point reads**:
the 1,025th new ID returns `InvalidBatch` before disk access, repeated
IDs remain usable even at the limit, invalid IDs return `InvalidId`,
and corrupt/I/O-failing point reads are not recorded. Snapshot capture
still clones O(distinct historical IDs) metadata; each tracked read ID
adds bounded set metadata until the view is dropped. Point tracking is
**only** through `TrackedSnapshot::get`; `DocumentStore::get` and
`DocumentSnapshot::get` do not participate.

**Not serializable transactions:** range/phantom/predicate reads cannot be
tracked by this wrapper, reads are not automatically discovered outside
its `get`, mutations are not staged or read-your-own-writes, and
there is no transaction ID, multi-frame commit, multiversion GC,
compaction, multi-writer isolation or distributed ACID guarantee.
Parent issue #6 remains open.

Regression:
`cargo test -p rystemdb-storage --locked --test tracked_snapshot`.

## Explicit point-read dependency validation for atomic batches (#83; child of #6)

`DocumentStore::commit_batch_checked(&snapshot, &[read_id, ...],
&[BatchMutation::Put/ Delete, ...])` is an **opt-in stricter**
version of `commit_batch`: the caller explicitly supplies all point keys
it read before deciding the mutations, including keys observed **absent**.
The writer compares the last-committed (segment,frame offset) for every
declared read dependency against the snapshot's generation ledger **before**
encoding the batch or performing I/O. An intervening change (including
same-value PUT or absent→PUT→DELETE) returns `StoreError::Conflict`
with no appended bytes, and none of the batch's changes are published.
Read and written IDs may overlap. Read dependencies can be unrelated
to the keys being written.

Up to **1,024 distinct read IDs** are supported. Duplicate read IDs
return `StoreError::InvalidBatch`; invalid identifiers return `InvalidId`,
and a snapshot from a foreign root returns `InvalidSnapshot`. A changed
unlisted ID does not conflict. The older `commit_batch` retains its
write-set-only validation by delegating to the same code with an empty
read set. Validation is metadata-only: **no payload reads or decoding**
until an actual storage operation is needed. Successful mutation batches
retain the same one-frame CRC, sync, atomic replay and publication
semantics defined above; **no wire/disk format change** is introduced.

**Not general serializable isolation:** the caller must accurately list
its point reads; `snapshot.get` does not automatically record them.
There is no protection against range/phantom queries, predicate conflicts,
multi-writer operation, durable transaction IDs or distributed commits.
Read-set checks do not create a multi-frame transaction. Parent #6
remains OPEN.

Regression: `cargo test -p rystemdb-storage --locked --test batch_read_set`.

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
- Experimental in-process read views, single-key optimistic conditional PUT/DELETE/INSERT and bounded atomic single-frame batches with retained tombstone generations; **no general transactional MVCC engine**, SQL, analytics projections, foreign-key enforcement, compaction, backup/restore or unbounded/multi-frame transactions.
- Operating-system and hardware flush semantics may differ. Power-loss tests and Windows directory durability remain necessary.
- CRC32 is not a cryptographic integrity guarantee; use trusted local storage.
- No guarantee for hostile filesystem modifications or arbitrary bit rot, beyond fail-closed checks for recognized corruption.

See [ADR 0002](../adr/0002-append-only-document-segments.md) and [Issue #2](https://github.com/KeyserDSoze/RystemDatabase/issues/2) for remaining acceptance gates.
