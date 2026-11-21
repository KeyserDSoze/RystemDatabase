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

Read/write/delete/rollover/restart, malformed input, corruption and truncated-tail behavior are covered by Cargo integration tests. The **feature-enabled real-process fault matrix** covers pre-append, torn synced payload, complete pre-sync write, post-sync/pre-index publication (PUT and DELETE), and post-directory-sync segment rotation. It is run on Linux, macOS and Windows.

## Crash semantics (test-only)

`cargo test -p rystemdb-storage --locked --features fault-injection --test crash_injection -- --test-threads=1` intentionally starts subprocesses which exit with code `97` at controlled storage boundaries, skipping Rust destructors. Only this test build reads `RYSTEMDB_TEST_CRASH_POINT`. Never build a release with `fault-injection` enabled. See [ADR 0003](../adr/0003-process-crash-fault-injection.md) and [crash test matrix](../tests/CRASH_GATE.md).

- Before append: existing acknowledged data survives; new value absent.
- Mid-frame with a persisted partial payload: only the incomplete tail of the **last** segment is repaired.
- Full write **before** sync: the new value may or may not replay, but must not be partial or invalid.
- After successful sync **before** in-memory publication: a complete PUT or DELETE must replay.
- After new segment creation and directory sync: prior documents survive and new empty segment is reusable.

These tests simulate **process termination**, not disk failure or sudden power loss.

## Explicit limitations

- **Advisory single-writer exclusion:** `rystemdb.writer.lock` is held with `File::try_lock` on the entire store lifetime, and released by the OS after normal or abrupt process exit. Two cooperative Rystem writers cannot open the same directory concurrently. Uncooperative tools or filesystems without reliable locks can still damage data.
- Rebuild-from-log on every startup (not yet an index engine).
- No MVCC, SQL, analytics projections, foreign-key enforcement, compaction, backup/restore or transactions spanning documents.
- Operating-system and hardware flush semantics may differ. Power-loss tests and Windows directory durability remain necessary.
- CRC32 is not a cryptographic integrity guarantee; use trusted local storage.
- No guarantee for hostile filesystem modifications or arbitrary bit rot, beyond fail-closed checks for recognized corruption.

See [ADR 0002](../adr/0002-append-only-document-segments.md) and [Issue #2](https://github.com/KeyserDSoze/RystemDatabase/issues/2) for remaining acceptance gates.
