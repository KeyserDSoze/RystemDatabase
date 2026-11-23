#!/usr/bin/env python3
"""#92 SQLite WAL/FULL *exploratory* baseline on exact Rust RDB fixture bytes.

Python sqlite3 BLOB reads are not the same semantic/runtime cost as Rust's
CRC-verified + typed RDB decode. Never publish these metrics as a fair winner
ranking without a native same-language/semantics runner and dedicated host.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import math
from pathlib import Path
import platform
import sqlite3
import sys
import tempfile
import time
from typing import Callable, TypeVar


MAX_FIXTURE_BYTES = 16 * 1024 * 1024
MAX_RECORDS = 256
MAX_OPERATIONS = 512
MAX_PAYLOAD = 1024
METRICS = {
    "put_sync", "get_hit_raw_blob_warm", "get_miss",
    "update_sync", "batch_2_sync", "delete_sync", "reopen_replay",
}
T = TypeVar("T")


@dataclass(frozen=True)
class Fixture:
    records: int
    operations: int
    seed: int
    payload_bytes: int
    initial: tuple[tuple[str, bytes], ...]
    read_ids: tuple[str, ...]
    updates: tuple[tuple[int, str, bytes], ...]
    batches: tuple[tuple[tuple[str, bytes], tuple[str, bytes]], ...]


def _keys(row: dict, kind: str, expected: set[str]) -> None:
    if type(row) is not dict or set(row) != expected:
        raise ValueError(f"{kind}: missing or unexpected fields")
    if row["kind"] != kind or type(row["schema"]) is not int or row["schema"] != 1:
        raise ValueError(f"{kind}: unsupported record kind/schema")


def _id(value: object) -> str:
    if (type(value) is not str or not value or len(value.encode("utf-8")) > 1024
            or "\x00" in value):
        raise ValueError("invalid document ID")
    return value


def _rdb_hex(value: object) -> bytes:
    if (type(value) is not str or len(value) % 2 != 0
            or len(value) > 128 * 1024 or not value):
        raise ValueError("invalid RDB hex frame length")
    try:
        raw = bytes.fromhex(value)
    except ValueError as error:
        raise ValueError("malformed RDB hex value") from error
    if len(raw) < 6 or raw[:5] != b"RDB\x00\x01" or raw[5] > 9:
        raise ValueError("invalid RDB v1 frame magic/version/tag")
    if raw[5] in (0, 1, 2) and len(raw) != 6:
        raise ValueError("invalid simple typed RDB frame length")
    if raw[5] in (3, 4) and len(raw) != 14:
        raise ValueError("invalid fixed-size typed RDB frame length")
    return raw


def load_fixture(path: Path) -> Fixture:
    """Fail closed on missing/extra rows, fields, keys, source schema or limits.

    Full nested codec validation lives in the Rust exporter. This Python
    loader deliberately does NOT pretend to perform typed RDB decoding.
    """
    path = Path(path)
    if path.stat().st_size > MAX_FIXTURE_BYTES:
        raise ValueError("fixture exceeds 16 MiB")
    try:
        rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
                if line.strip()]
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid fixture NDJSON") from error
    if not rows:
        raise ValueError("empty fixture")
    run = rows[0]
    _keys(run, "fixture_run", {
        "schema", "kind", "seed", "records", "operations",
        "payload_bytes", "engine", "value_encoding",
    })
    if run["engine"] != "rystemdb-storage" or run["value_encoding"] != "rdb_v1_frame_hex":
        raise ValueError("fixture does not contain native RDB frames")
    for key, low, high in (
        ("seed", 0, (1 << 64) - 1),
        ("records", 1, MAX_RECORDS),
        ("operations", 1, MAX_OPERATIONS),
        ("payload_bytes", 0, MAX_PAYLOAD),
    ):
        val = run[key]
        if type(val) is not int or not low <= val <= high:
            raise ValueError(f"invalid fixture parameter {key}")
    records, ops = run["records"], run["operations"]
    if len(rows) != 1 + records + 3 * ops:
        raise ValueError("incorrect fixture row count")

    pos = 1
    initial: list[tuple[str, bytes]] = []
    seen = set()
    for i in range(records):
        row = rows[pos]
        pos += 1
        _keys(row, "initial", {"schema", "kind", "id", "value_hex"})
        id_ = _id(row["id"])
        if id_ != f"bench/{i:08}" or id_ in seen:
            raise ValueError("duplicate or out-of-order original document ID")
        seen.add(id_)
        initial.append((id_, _rdb_hex(row["value_hex"])))

    read_ids: list[str] = []
    for i in range(ops):
        row = rows[pos]
        pos += 1
        _keys(row, "read", {"schema", "kind", "sequence", "id"})
        if type(row["sequence"]) is not int or row["sequence"] != i:
            raise ValueError("missing or out-of-order read sequence")
        id_ = _id(row["id"])
        if id_ not in seen:
            raise ValueError("read references a missing original ID")
        read_ids.append(id_)

    updates: list[tuple[int, str, bytes]] = []
    for i in range(ops):
        row = rows[pos]
        pos += 1
        _keys(row, "update", {"schema", "kind", "sequence", "id", "value_hex"})
        if type(row["sequence"]) is not int or row["sequence"] != i:
            raise ValueError("missing or out-of-order update sequence")
        id_ = _id(row["id"])
        if id_ != read_ids[i]:
            raise ValueError("update and read do not use same seeded key sequence")
        updates.append((i, id_, _rdb_hex(row["value_hex"])))

    batches: list[tuple[tuple[str, bytes], tuple[str, bytes]]] = []
    for i in range(ops):
        row = rows[pos]
        pos += 1
        _keys(row, "batch", {
            "schema", "kind", "sequence", "id_a", "value_a_hex",
            "id_b", "value_b_hex",
        })
        if type(row["sequence"]) is not int or row["sequence"] != i:
            raise ValueError("missing or out-of-order batch sequence")
        a, b = _id(row["id_a"]), _id(row["id_b"])
        if a != f"batch/{i:08}/a" or b != f"batch/{i:08}/b" or a == b:
            raise ValueError("invalid or non-distinct pair of batch IDs")
        batches.append(((a, _rdb_hex(row["value_a_hex"])),
                        (b, _rdb_hex(row["value_b_hex"]))))
    return Fixture(records, ops, run["seed"], run["payload_bytes"],
                   tuple(initial), tuple(read_ids), tuple(updates), tuple(batches))


def _measure(name: str, count: int, operation: Callable[[int], T],
             verify: Callable[[int, T], bool]) -> dict:
    samples = []
    wall = time.perf_counter_ns()
    for i in range(count):
        start = time.perf_counter_ns()
        result = operation(i)
        samples.append(time.perf_counter_ns() - start)
        if not verify(i, result):
            raise ValueError(f"SQLite returned incorrect data for {name} operation {i}")
    elapsed = time.perf_counter_ns() - wall
    ordered = sorted(samples)

    def rank(p: int) -> int:
        return ordered[max(0, (len(ordered) * p + 99) // 100 - 1)]

    ns = [rank(p) for p in (50, 95, 99)] + [ordered[-1]]
    return {
        "schema": 1, "kind": "metric", "name": name, "count": len(samples),
        "p50_ns": ns[0], "p95_ns": ns[1], "p99_ns": ns[2], "max_ns": ns[3],
        "p50_ms": round(ns[0] / 1_000_000, 6),
        "p95_ms": round(ns[1] / 1_000_000, 6),
        "p99_ms": round(ns[2] / 1_000_000, 6),
        "max_ms": round(ns[3] / 1_000_000, 6),
        "loop_wall_ms": round(elapsed / 1_000_000, 6),
        "loop_wall_ops_per_sec": round(count * 1_000_000_000 / elapsed, 3)
        if elapsed else 0.0,
    }


def _open_wal_full(path: Path) -> sqlite3.Connection:
    # isolation_level=None explicitly leaves SQLite in autocommit mode,
    # so each write below has one manually controlled BEGIN/COMMIT.
    db = sqlite3.connect(path, isolation_level=None, cached_statements=128)
    mode = db.execute("PRAGMA journal_mode=WAL").fetchone()[0].lower()
    db.execute("PRAGMA synchronous=FULL")
    synchronous = db.execute("PRAGMA synchronous").fetchone()[0]
    if mode != "wal" or synchronous != 2:
        db.close()
        raise ValueError("SQLite WAL+FULL durability configuration not applied")
    return db


def _transaction(db: sqlite3.Connection, steps: tuple[tuple[str, tuple], ...]) -> int:
    db.execute("BEGIN IMMEDIATE")
    try:
        changed = 0
        for sql, params in steps:
            cursor = db.execute(sql, params)
            changed += cursor.rowcount
        db.execute("COMMIT")
        return changed
    except BaseException:
        db.execute("ROLLBACK")
        raise


def run_sqlite(fixture: Fixture, db_path: Path) -> list[dict]:
    """Run exactly the Rust fixture's key/value operations on real SQLite.

    Single Python process, actual disk-backed WAL/FULL transactions.
    SQL returns bytes (opaque RDB BLOB), NOT a typed/CRC-checked Rystem read.
    """
    db_path = Path(db_path)
    if db_path.exists():
        raise ValueError("SQLite benchmark must start with a fresh database path")
    db = _open_wal_full(db_path)
    metrics: list[dict] = []
    start_wall = time.perf_counter_ns()
    original = dict(fixture.initial)
    current = dict(original)
    read_sql = "SELECT payload FROM documents WHERE id = ?"
    insert_sql = "INSERT INTO documents (id, payload) VALUES (?, ?)"
    update_sql = "UPDATE documents SET payload = ? WHERE id = ?"
    delete_sql = "DELETE FROM documents WHERE id = ?"

    def lookup(id_: str) -> bytes | None:
        row = db.execute(read_sql, (id_,)).fetchone()
        return row[0] if row is not None else None

    def put(i: int) -> int:
        id_, value = fixture.initial[i]
        return _transaction(db, ((insert_sql, (id_, value)),))

    def update(i: int) -> int:
        _, id_, value = fixture.updates[i]
        changed = _transaction(db, ((update_sql, (value, id_)),))
        current[id_] = value
        return changed

    def batch(i: int) -> int:
        a, b = fixture.batches[i]
        return _transaction(db, (
            (insert_sql, (a[0], a[1])),
            (insert_sql, (b[0], b[1])),
        ))

    try:
        db.execute("CREATE TABLE documents(id TEXT PRIMARY KEY NOT NULL, payload BLOB NOT NULL) WITHOUT ROWID")
        metrics.append(_measure("put_sync", fixture.records, put,
            lambda i, count: count == 1
                and lookup(fixture.initial[i][0]) == fixture.initial[i][1]))

        for id_, value in fixture.initial[:min(64, fixture.records)]:
            if lookup(id_) != value:
                raise ValueError("SQLite warmup did not retrieve original bytes")

        metrics.append(_measure("get_hit_raw_blob_warm", fixture.operations,
            lambda i: lookup(fixture.read_ids[i]),
            lambda i, result: result == original[fixture.read_ids[i]]))

        metrics.append(_measure("get_miss", fixture.operations,
            lambda i: lookup(f"never-present/{i:08}"),
            lambda _i, result: result is None))

        metrics.append(_measure("update_sync", fixture.operations, update,
            lambda i, count: count == 1
                and lookup(fixture.updates[i][1]) == fixture.updates[i][2]))

        for id_, value in current.items():
            if lookup(id_) != value:
                raise ValueError("SQLite final updated corpus mismatch")

        metrics.append(_measure("batch_2_sync", fixture.operations, batch,
            lambda i, count: count == 2
                and all(lookup(id_) == value for id_, value in fixture.batches[i])))

        for a, b in fixture.batches:
            if lookup(a[0]) != a[1] or lookup(b[0]) != b[1]:
                raise ValueError("SQLite final 2-key transaction replay mismatch")

        metrics.append(_measure("delete_sync", fixture.records,
            lambda i: _transaction(db, ((delete_sql, (fixture.initial[i][0],)),)),
            lambda i, count: count == 1 and lookup(fixture.initial[i][0]) is None))

        if db.execute("SELECT COUNT(*) FROM documents").fetchone()[0] != fixture.operations * 2:
            raise ValueError("SQLite unexpected final row count")

        # Account for the WAL, SHM and main db while active. SQLite may
        # checkpoint and remove WAL/SHM on final connection close.
        footprints = [Path(str(db_path) + suffix) for suffix in ("", "-wal", "-shm")]
        disk_bytes = sum(p.stat().st_size for p in footprints if p.exists())
        files = sum(1 for p in footprints if p.exists())
        db.close()

        def reopen(_i: int) -> sqlite3.Connection:
            return sqlite3.connect(db_path, isolation_level=None)

        def verify_reopen(_i: int, connection: sqlite3.Connection) -> bool:
            try:
                first_a, first_b = fixture.batches[0]
                return (
                    connection.execute(
                        "SELECT COUNT(*) FROM documents"
                    ).fetchone()[0] == fixture.operations * 2
                    and connection.execute(read_sql, (first_a[0],)).fetchone()[0] == first_a[1]
                    and connection.execute(read_sql, (first_b[0],)).fetchone()[0] == first_b[1]
                    and connection.execute(read_sql, (fixture.initial[0][0],)).fetchone() is None
                )
            finally:
                connection.close()

        metrics.append(_measure("reopen_replay", 1, reopen, verify_reopen))
    finally:
        db.close()

    report: list[dict] = [{
        "schema": 1, "kind": "run", "engine": "sqlite",
        "engine_version": sqlite3.sqlite_version,
        "driver": "python_stdlib_sqlite3",
        "driver_version": platform.python_version(),
        "mode": "embedded_single_writer",
        "sqlite_journal_mode": "wal",
        "sqlite_synchronous": "FULL",
        "durability": "WAL_FULL_explicit_commit_each_write",
        "payload_semantics": "opaque_rdb_blob_no_typed_crc_decode",
        "read_cache": "warmed",
        "timing_unit": "nanoseconds",
        "seed": fixture.seed, "records": fixture.records,
        "operations": fixture.operations, "payload_bytes": fixture.payload_bytes,
        "os": sys.platform, "arch": platform.machine(),
    }]
    report.extend(metrics)
    report.append({
        "schema": 1, "kind": "summary",
        "disk_bytes": disk_bytes, "sqlite_file_count": files,
        "elapsed_ms": round((time.perf_counter_ns() - start_wall) / 1_000_000, 6),
    })
    return report


def validate_sqlite_report(report: list[dict], fixture: Fixture) -> None:
    if len(report) != len(METRICS) + 2:
        raise ValueError("incorrect number of SQLite metric records")
    run, *metrics, summary = report
    if (run.get("kind") != "run" or run.get("schema") != 1
            or run.get("engine") != "sqlite"
            or run.get("sqlite_journal_mode") != "wal"
            or run.get("sqlite_synchronous") != "FULL"
            or run.get("payload_semantics") != "opaque_rdb_blob_no_typed_crc_decode"
            or run.get("records") != fixture.records
            or run.get("operations") != fixture.operations
            or run.get("seed") != fixture.seed
            or run.get("payload_bytes") != fixture.payload_bytes):
        raise ValueError("SQLite report metadata/parity mismatch")
    names = [m.get("name") for m in metrics]
    if set(names) != METRICS or len(names) != len(METRICS):
        raise ValueError("duplicate or missing SQLite workload")
    for m in metrics:
        name = m["name"]
        count = (1 if name == "reopen_replay" else
                 fixture.records if name in ("put_sync", "delete_sync") else
                 fixture.operations)
        if m.get("kind") != "metric" or m.get("schema") != 1 or m.get("count") != count:
            raise ValueError(f"SQLite bad metric count: {name}")
        ns = [m.get(f"{p}_ns") for p in ("p50", "p95", "p99", "max")]
        if any(type(x) is not int or x < 0 for x in ns) or ns != sorted(ns):
            raise ValueError(f"SQLite invalid latency distribution: {name}")
        for p, n in zip(("p50", "p95", "p99", "max"), ns):
            ms = m.get(f"{p}_ms")
            if (not isinstance(ms, (int, float)) or not math.isfinite(ms)
                    or abs(ms - n / 1_000_000) > 0.0000011):
                raise ValueError(f"SQLite invalid ms field: {name}/{p}")
        for key in ("loop_wall_ms", "loop_wall_ops_per_sec"):
            value = m.get(key)
            if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
                raise ValueError(f"SQLite nonfinite throughput: {name}/{key}")
    if (summary.get("kind") != "summary" or summary.get("schema") != 1
            or type(summary.get("disk_bytes")) is not int or summary["disk_bytes"] <= 0
            or type(summary.get("sqlite_file_count")) is not int
            or summary["sqlite_file_count"] < 1):
        raise ValueError("SQLite missing disk/WAL footprint")


def _cli() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rystem-report", type=Path)
    args = parser.parse_args()
    fixture = load_fixture(args.fixture)
    if args.rystem_report:
        first = json.loads(args.rystem_report.read_text(encoding="utf-8").splitlines()[0])
        if (first.get("engine") != "rystemdb-storage"
                or any(first.get(k) != getattr(fixture, k)
                       for k in ("seed", "records", "operations", "payload_bytes"))):
            raise ValueError("SQLite and Rystem metadata do not describe same workload")
    with tempfile.TemporaryDirectory(prefix="rystem-sqlite-perf-") as name:
        report = run_sqlite(fixture, Path(name) / "comparison.sqlite3")
    validate_sqlite_report(report, fixture)
    ndjson = "\n".join(json.dumps(row, separators=(",", ":"), allow_nan=False)
                      for row in report) + "\n"
    args.output.write_text(ndjson, encoding="utf-8")
    print(ndjson, end="")


if __name__ == "__main__":
    _cli()
