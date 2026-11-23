#!/usr/bin/env python3
"""#92 TDD RED: SQLite baseline must use exact RDB byte fixture and WAL/FULL.

The benchmark may NEVER be treated as equal to the Rust typed RDB engine:
Python sqlite3 decodes raw BLOBs and does not verify an RYSF frame CRC.
"""
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from benchmark_sqlite import load_fixture, run_sqlite, validate_sqlite_report


RDB_NULL = "524442000100"
RDB_TRUE = "524442000102"
RDB_FALSE = "524442000101"
RDB_INT_1 = "5244420001030000000000000001"
RDB_INT_2 = "5244420001030000000000000002"


def example_rows() -> list[dict]:
    """Same logical schema written by the Rust fixture exporter."""
    return [
        {"schema": 1, "kind": "fixture_run", "seed": 42,
         "records": 2, "operations": 2, "payload_bytes": 0,
         "engine": "rystemdb-storage", "value_encoding": "rdb_v1_frame_hex"},
        {"schema": 1, "kind": "initial", "id": "bench/00000000",
         "value_hex": RDB_NULL},
        {"schema": 1, "kind": "initial", "id": "bench/00000001",
         "value_hex": RDB_TRUE},
        {"schema": 1, "kind": "read", "sequence": 0, "id": "bench/00000001"},
        {"schema": 1, "kind": "read", "sequence": 1, "id": "bench/00000000"},
        {"schema": 1, "kind": "update", "sequence": 0, "id": "bench/00000001",
         "value_hex": RDB_INT_1},
        {"schema": 1, "kind": "update", "sequence": 1, "id": "bench/00000000",
         "value_hex": RDB_INT_2},
        {"schema": 1, "kind": "batch", "sequence": 0,
         "id_a": "batch/00000000/a", "value_a_hex": RDB_INT_1,
         "id_b": "batch/00000000/b", "value_b_hex": RDB_NULL},
        {"schema": 1, "kind": "batch", "sequence": 1,
         "id_a": "batch/00000001/a", "value_a_hex": RDB_INT_2,
         "id_b": "batch/00000001/b", "value_b_hex": RDB_NULL},
    ]


def write_fixture(root: Path, rows: list[dict]) -> Path:
    file = root / "fixture.ndjson"
    file.write_text("\n".join(json.dumps(r) for r in rows) + "\n", encoding="utf-8")
    return file


class FixtureContract(unittest.TestCase):
    def test_valid_fixture_parses_exact_raw_rdb_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            fixture = load_fixture(write_fixture(Path(name), example_rows()))
            self.assertEqual(fixture.records, 2)
            self.assertEqual(fixture.operations, 2)
            self.assertEqual(fixture.initial[0][1], bytes.fromhex(RDB_NULL))
            self.assertEqual(fixture.read_ids, ("bench/00000001", "bench/00000000"))
            self.assertEqual(fixture.updates[0][1], "bench/00000001")
            self.assertEqual(fixture.batches[1][0][1], bytes.fromhex(RDB_INT_2))

    def test_rejects_duplicate_initial_ids(self) -> None:
        rows = example_rows()
        rows[2]["id"] = rows[1]["id"]
        with tempfile.TemporaryDirectory() as name:
            with self.assertRaises(ValueError):
                load_fixture(write_fixture(Path(name), rows))

    def test_rejects_missing_sequence_and_extra_rows(self) -> None:
        rows = example_rows()
        rows[3]["sequence"] = 7
        with tempfile.TemporaryDirectory() as name:
            with self.assertRaises(ValueError):
                load_fixture(write_fixture(Path(name), rows))
            with self.assertRaises(ValueError):
                load_fixture(write_fixture(Path(name), example_rows() + [rows[-1]]))

    def test_rejects_bad_hex_missing_magic_and_trailing_fields(self) -> None:
        for replacement in ("123", "not-hex", "0000000000", "5244420001"):
            rows = example_rows()
            rows[1]["value_hex"] = replacement
            with tempfile.TemporaryDirectory() as name:
                with self.assertRaises(ValueError):
                    load_fixture(write_fixture(Path(name), rows))
        rows = example_rows()
        rows[0]["untrusted"] = "extra"
        with tempfile.TemporaryDirectory() as name:
            with self.assertRaises(ValueError):
                load_fixture(write_fixture(Path(name), rows))

    def test_rejects_empty_or_oversized_ids_and_overlimit_counts(self) -> None:
        for invalid in ("", "x" * 1025):
            rows = example_rows()
            rows[1]["id"] = invalid
            with tempfile.TemporaryDirectory() as name:
                with self.assertRaises(ValueError):
                    load_fixture(write_fixture(Path(name), rows))
        rows = example_rows()
        rows[0]["operations"] = 99999
        with tempfile.TemporaryDirectory() as name:
            with self.assertRaises(ValueError):
                load_fixture(write_fixture(Path(name), rows))

    def test_full_sqlite_wal_full_acknowledged_transactions_and_replay(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            path = Path(name)
            fixture = load_fixture(write_fixture(path, example_rows()))
            report = run_sqlite(fixture, path / "sqlite.db")
            validate_sqlite_report(report, fixture)
            run = report[0]
            self.assertEqual(run["sqlite_journal_mode"], "wal")
            self.assertEqual(run["sqlite_synchronous"], "FULL")
            self.assertEqual(run["payload_semantics"], "opaque_rdb_blob_no_typed_crc_decode")
            self.assertEqual(run["engine"], "sqlite")
            metrics = {row["name"]: row for row in report if row["kind"] == "metric"}
            self.assertEqual(metrics["put_sync"]["count"], 2)
            self.assertEqual(metrics["get_hit_raw_blob_warm"]["count"], 2)
            self.assertEqual(metrics["get_miss"]["count"], 2)
            self.assertEqual(metrics["update_sync"]["count"], 2)
            self.assertEqual(metrics["batch_2_sync"]["count"], 2)
            self.assertEqual(metrics["delete_sync"]["count"], 2)
            self.assertEqual(metrics["reopen_replay"]["count"], 1)
            self.assertGreater(report[-1]["disk_bytes"], 0)

    def test_report_rejects_missing_metric_and_disordered_percentiles(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            path = Path(name)
            fixture = load_fixture(write_fixture(path, example_rows()))
            report = run_sqlite(fixture, path / "sqlite.db")
            del report[-2]
            with self.assertRaises(ValueError):
                validate_sqlite_report(report, fixture)
            report = run_sqlite(fixture, path / "sqlite2.db")
            report[1]["p95_ns"] = -1
            with self.assertRaises(ValueError):
                validate_sqlite_report(report, fixture)


if __name__ == "__main__":
    unittest.main(verbosity=2)
