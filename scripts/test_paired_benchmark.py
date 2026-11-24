#!/usr/bin/env python3
"""#96 RED: repeatable alternating A/B with exact metadata and raw-run proof.

No assertion ever compares elapsed milliseconds to an arbitrary limit.
"""
from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from check_storage_benchmark import sample as rystem_sample
from check_native_sqlite_benchmark import sample as sqlite_sample
from paired_benchmark import (
    alternating_orders,
    aggregate_runs,
    run_comparison,
    verify_pair,
)


def fixtures(repetition: int = 0) -> tuple[list[dict], list[dict]]:
    rust = [json.loads(x) for x in rystem_sample().splitlines()]
    sqlite = sqlite_sample()
    sqlite[0].update({
        "records": 4, "operations": 7, "payload_bytes": 128,
        "read_cache": "warmed",
    })
    rust[0]["read_cache"] = "warmed"
    for m in sqlite[1:-1]:
        m["count"] = 1 if m["name"] == "reopen_replay" else (
            4 if m["name"] in {"put_sync", "delete_sync"} else 7
        )
    for engine in (rust, sqlite):
        for metric in engine[1:-1]:
            # Identical deterministic values except each run varies in a
            # controlled, non-sorted order. Their median is not the minimum.
            ns = (1000 + [40, 10, 100, 20, 30][repetition])
            if engine is sqlite:
                ns *= 2
            for key, extra in (("p50", 0), ("p95", 20), ("p99", 40), ("max", 60)):
                metric[f"{key}_ns"] = ns + extra
                metric[f"{key}_ms"] = (ns + extra) / 1_000_000
    return rust, sqlite


class ScheduleTests(unittest.TestCase):
    def test_five_runs_balanced_and_deterministic(self):
        first = alternating_orders(5, 42)
        self.assertEqual(first, alternating_orders(5, 42))
        self.assertEqual(len(first), 5)
        self.assertEqual(set(first), {("rystem", "sqlite"), ("sqlite", "rystem")})
        self.assertEqual([x[0] for x in first],
                         ["rystem", "sqlite", "rystem", "sqlite", "rystem"])
        self.assertEqual([x[0] for x in alternating_orders(5, 43)],
                         ["sqlite", "rystem", "sqlite", "rystem", "sqlite"])

    def test_six_runs_fully_balanced_and_bad_repeats_rejected(self):
        order = alternating_orders(6, 42)
        self.assertEqual(sum(first == "rystem" for first, _ in order), 3)
        for reps in (0, 1, 4, 26, -3, True):
            with self.subTest(repeats=reps), self.assertRaises(ValueError):
                alternating_orders(reps, 42)


class ContractTests(unittest.TestCase):
    def test_matching_full_reports_validate(self):
        rust, sqlite = fixtures()
        verify_pair(rust, sqlite)
        self.assertEqual(rust[0]["seed"], sqlite[0]["seed"])

    def test_metadata_drift_fails_closed(self):
        for field, bad in (("seed", 99), ("records", 1), ("payload_bytes", 1024)):
            rust, sqlite = fixtures()
            sqlite[0][field] = bad
            with self.subTest(field=field), self.assertRaises(ValueError):
                verify_pair(rust, sqlite)

    def test_corrupt_or_missing_report_does_not_aggregate(self):
        rust, sqlite = fixtures()
        sqlite[1]["p99_ns"] = -1
        with self.assertRaises(ValueError):
            aggregate_runs([(rust, sqlite)] * 5)
        rust, sqlite = fixtures()
        del sqlite[2]
        with self.assertRaises(ValueError):
            aggregate_runs([(rust, sqlite)] * 5)

    def test_median_of_run_percentiles_not_min_or_pooled_samples(self):
        paired = [fixtures(i) for i in range(5)]
        output = aggregate_runs(paired)
        self.assertEqual(len(output), 14, "seven comparable scenarios, two engines")
        put = next(row for row in output
                   if row["scenario"] == "put_sync" and row["engine"] == "rystem")
        self.assertEqual(put["runs"], 5)
        self.assertEqual(put["median_run_p50_ns"], 1030)
        self.assertEqual(put["min_run_p50_ns"], 1010)
        self.assertEqual(put["max_run_p50_ns"], 1100)
        self.assertEqual(put["median_run_p99_ns"], 1070)
        self.assertEqual(put["min_run_p99_ns"], 1050)
        sqlite = next(row for row in output
                      if row["scenario"] == "put_sync" and row["engine"] == "sqlite")
        self.assertEqual(sqlite["median_run_p50_ns"], 2060)
        self.assertEqual(sqlite["max_run_p50_ns"], 2200)

    def test_read_and_batch_integrity_differences_disclosed(self):
        output = aggregate_runs([fixtures(i) for i in range(5)])
        get = [x for x in output if x["scenario"] == "typed_get_warm"]
        batch = [x for x in output if x["scenario"] == "two_key_commit_sync"]
        self.assertEqual(len(get), 2)
        self.assertTrue(all(x["semantics"] == "non_equivalent_outer_rysf_crc" for x in get))
        self.assertTrue(all(x["semantics"] == "non_equivalent_staging_overhead" for x in batch))
        self.assertFalse(any(x.get("winner") for x in output))

    def test_refuses_nonempty_directory_or_missing_binaries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "already-exists").write_text("x", encoding="utf-8")
            with self.assertRaises((ValueError, FileNotFoundError)):
                run_comparison(Path("/missing/rystem"), Path("/missing/sqlite"),
                               root, 5, 42, 4, 7, 128)
            self.assertEqual(sorted(p.name for p in root.iterdir()), ["already-exists"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
