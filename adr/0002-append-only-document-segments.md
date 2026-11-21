# ADR 0002 — Experimental append-only document segments

Status: Accepted for experimental single-process storage — 2026-11-21

## Decision

- A **self-describing** `rystemdb-core` typed value is embedded as a payload inside a v1 `RYSF` record.
- Each frame: 4-byte `RYSF` magic, 1-byte version=1, 1-byte op (PUT=1, DELETE=2), 2-byte reserved=0, 4-byte big-endian UTF-8 ID length, 4-byte big-endian payload length, 4-byte big-endian IEEE CRC32, followed by ID bytes and binary payload bytes.
- CRC32 covers header bytes [4..16] + ID + payload, excluding magic and checksum itself.
- IDs must be 1..1024 bytes; payloads must satisfy the codec's 64 MiB frame bound. Tombstones have zero payload.
- Segment filenames are monotonically numbered `segment-00000001.rdb`, etc. Records are append-only, and each PUT or DELETE is synced before the in-memory ID → offset index is updated. Rollover and directory sync occur before new records are referenced (directory sync on Unix only).
- On restart, scan every frame and rebuild the index. **Only an incomplete frame in the last segment** may be truncated; a CRC mismatch, invalid header, missing segment or incomplete frame in an older segment always fails closed.
- Read paths validate the CRC and codec again, so post-open tampering is detected.
- A write or sync failure poisons the open writer. The caller must stop writing and reopen to replay the log.

## Non-goals and risks

- **No multiprocess file locking yet.** Only one caller/process may open for writing; concurrent process access is unsupported and can corrupt logical state.
- The in-memory ID map is not a separately persisted transactional index; startup replay is O(number of frames).
- There are no multi-document transactions, MVCC, guaranteed atomic compaction, document revisions, or foreign-key constraints.
- No proof of Windows directory-entry persistence after power loss. A successful `sync_data` call is not the same as verified physical durability across all filesystems/hardware.
- CRC32 detects accidental damage, but does not authenticate data; encryption and integrity against an adversary are future work.
- Recovery of a partially written trailing frame does not imply safe recovery from arbitrary device corruption.
- Restarted code must support the file format version; changes require migration tests and a new version.

## TDD

- [RED 45e9fd2](https://github.com/KeyserDSoze/RystemDatabase/commit/45e9fd2459b7f76f8653f661463ca6e99c569afb): 8 new failing storage contract tests.
- [RED CI 44913469078](https://github.com/KeyserDSoze/RystemDatabase/actions/runs/44913469078): tests failed on Linux/Windows/macOS.
- [Implementation 23ae5ee](https://github.com/KeyserDSoze/RystemDatabase/commit/23ae5eeb95f2b072a6653b0ee7599e9ce4b71073): initial implementation, pending final checks.
