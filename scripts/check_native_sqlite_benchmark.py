#!/usr/bin/env python3
"""Independent, no-dependency validation of the native RUST sqlite report.

No latency thresholds from shared GitHub runners; typed-vs-CRC mismatch is
always explicitly declared.
"""
from __future__ import annotations
import argparse
import json
import math
from pathlib import Path
import unittest

METRICS = {
    "put_sync", "get_hit_typed_rdb_warm", "get_miss",
    "update_sync", "batch_2_sync", "delete_sync", "reopen_replay",
}


def check(rows: list[dict], rust_run: dict | None = None) -> None:
    if len(rows) != len(METRICS) + 2:
        raise ValueError("wrong number of native benchmark rows")
    run, *metrics, summary = rows
    if (run.get("kind") != "run" or run.get("schema") != 1
            or run.get("engine") != "sqlite"
            or run.get("driver") != "native_rust_rusqlite"
            or run.get("sqlite_journal_mode") != "wal"
            or run.get("sqlite_synchronous") != "FULL"
            or run.get("payload_semantics") != "rdb_v1_typed_decode_no_rysf_outer_crc"
            or not isinstance(run.get("engine_version"), str)
            or not run["engine_version"]
            or run.get("timing_unit") != "nanoseconds"):
        raise ValueError("invalid native Rust SQLite run metadata or integrity disclosure")
    for field in ("seed", "records", "operations", "payload_bytes"):
        if type(run.get(field)) is not int:
            raise ValueError(f"invalid {field}")
        if rust_run is not None and rust_run.get(field) != run[field]:
            raise ValueError(f"Rystem and native SQLite {field} mismatch")
    names = [m.get("name") for m in metrics]
    if set(names) != METRICS or len(set(names)) != len(METRICS):
        raise ValueError("missing or duplicate native workload")
    for m in metrics:
        name = m["name"]
        count = 1 if name == "reopen_replay" else (
            run["records"] if name in {"put_sync", "delete_sync"}
            else run["operations"]
        )
        if m.get("kind") != "metric" or m.get("schema") != 1 or m.get("count") != count:
            raise ValueError(f"invalid metric count or kind: {name}")
        vals = [m.get(k + "_ns") for k in ("p50", "p95", "p99", "max")]
        if any(type(n) is not int or n < 0 for n in vals) or vals != sorted(vals):
            raise ValueError(f"invalid latency ordering: {name}")
        for key, value in zip(("p50", "p95", "p99", "max"), vals):
            ms = m.get(key + "_ms")
            if (not isinstance(ms, (int, float)) or not math.isfinite(ms)
                    or abs(ms - value / 1_000_000) > 0.0000011):
                raise ValueError(f"wrong milliseconds: {name}/{key}")
        for key in ("loop_wall_ms", "loop_wall_ops_per_sec"):
            value = m.get(key)
            if (not isinstance(value, (int, float)) or
                    not math.isfinite(value) or value < 0):
                raise ValueError(f"invalid wall time/throughput: {name}/{key}")
    if (summary.get("kind") != "summary" or summary.get("schema") != 1 or
            type(summary.get("disk_bytes")) is not int or summary["disk_bytes"] < 1 or
            type(summary.get("sqlite_file_count")) is not int or
            summary["sqlite_file_count"] < 1):
        raise ValueError("missing real SQLite database/WAL/SHM footprint")


def sample() -> list[dict]:
    base = {"schema": 1, "kind": "run", "engine": "sqlite",
            "driver": "native_rust_rusqlite", "engine_version": "3.50.0",
            "sqlite_journal_mode": "wal", "sqlite_synchronous": "FULL",
            "payload_semantics": "rdb_v1_typed_decode_no_rysf_outer_crc",
            "timing_unit": "nanoseconds", "records": 3, "operations": 4,
            "seed": 42, "payload_bytes": 64}
    rows = []
    for name in sorted(METRICS):
        count = 1 if name == "reopen_replay" else (
            3 if name in {"put_sync", "delete_sync"} else 4
        )
        rows.append({"schema": 1, "kind": "metric", "name": name, "count": count,
                     "p50_ns": 100, "p95_ns": 200, "p99_ns": 300, "max_ns": 400,
                     "p50_ms": .0001, "p95_ms": .0002, "p99_ms": .0003,
                     "max_ms": .0004, "loop_wall_ms": 2.,
                     "loop_wall_ops_per_sec": 1500.})
    return [base, *rows, {"schema": 1, "kind": "summary",
                          "disk_bytes": 8192, "sqlite_file_count": 3}]


class ValidatorTests(unittest.TestCase):
    def test_valid(self):
        check(sample())

    def test_wrong_integrity_claim(self):
        rows = sample()
        rows[0]["payload_semantics"] = "full_rysf_crc_checked"
        with self.assertRaises(ValueError):
            check(rows)

    def test_mismatched_rust_fixture(self):
        with self.assertRaises(ValueError):
            check(sample(), {"seed": 99})

    def test_missing_metric(self):
        with self.assertRaises(ValueError):
            check(sample()[:-2] + sample()[-1:])

    def test_invalid_p99(self):
        rows = sample()
        rows[1]["p99_ns"] = 1
        with self.assertRaises(ValueError):
            check(rows)

    def test_nonfinite_time(self):
        rows = sample()
        rows[1]["loop_wall_ops_per_sec"] = float("nan")
        with self.assertRaises(ValueError):
            check(rows)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--native-report", type=Path)
    parser.add_argument("--rystem-report", type=Path)
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[__file__], verbosity=2)
    else:
        if not args.native_report or not args.rystem_report:
            parser.error("--native-report and --rystem-report required")
        native = [json.loads(x) for x in args.native_report.read_text().splitlines() if x.strip()]
        original = json.loads(args.rystem_report.read_text().splitlines()[0])
        check(native, original)
        print("native Rust SQLite typed benchmark validated: 7 workloads")
