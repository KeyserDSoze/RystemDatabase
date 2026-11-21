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

## Explicit limitations

- **Single writer only, without OS-enforced locking yet.** Never open the same directory for mutation in two independent processes. Track as a safety follow-up before public data APIs.
- Rebuild-from-log on every startup (not yet an index engine).
- No MVCC, SQL, analytics projections, foreign-key enforcement, compaction, backup/restore or transactions spanning documents.
- Operating-system and hardware flush semantics may differ. Power-loss tests and Windows directory durability remain necessary.
- CRC32 is not a cryptographic integrity guarantee; use trusted local storage.
- No guarantee for hostile filesystem modifications or arbitrary bit rot, beyond fail-closed checks for recognized corruption.

See [ADR 0002](../adr/0002-append-only-document-segments.md) and [Issue #2](https://github.com/KeyserDSoze/RystemDatabase/issues/2) for remaining acceptance gates.
