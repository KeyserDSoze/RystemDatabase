#!/usr/bin/env python3
"""#88 Validate machine-readable storage latency reports without time thresholds.

Checks schema, percentile ordering, workload coverage and math, not absolute
milliseconds. Shared CI runner measurements are diagnostic, not SLA proof.
"""
from __future__ import annotations

import json
import math
import pathlib
import sys
import unittest

EXPECTED = {
    "put_sync",
    "get_hit_typed_crc_warm",
    "get_miss",
    "get_two_batch_members_sequential_warm",
    "get_two_batch_members_grouped_warm",
    "snapshot_capture",
    "update_sync",
    "staged_batch_2_sync",
    "delete_sync",
    "reopen_replay",
}


def validate(text: str) -> list[dict]:
    objects = [json.loads(line) for line in text.splitlines() if line.strip()]
    if len(objects) != len(EXPECTED) + 2:
        raise ValueError("exactly one metadata, ten metrics, one summary required")
    if any(item.get("schema") != 1 for item in objects):
        raise ValueError("unsupported schema")
    run, *metrics, summary = objects
    if run.get("kind") != "run" or summary.get("kind") != "summary":
        raise ValueError("missing run/summary boundaries")
    for key in ("engine", "engine_version", "mode", "durability", "os", "arch"):
        if not isinstance(run.get(key), str) or not run[key]:
            raise ValueError(f"missing metadata: {key}")
    if run["engine"] != "rystemdb-storage" or run["mode"] != "embedded_single_writer":
        raise ValueError("wrong engine/mode")
    for key in ("records", "operations"):
        if type(run.get(key)) is not int or run[key] < 1:
            raise ValueError(f"invalid count {key}")
    if type(run.get("seed")) is not int or type(run.get("payload_bytes")) is not int:
        raise ValueError("invalid corpus parameters")
    if run.get("timing_unit") != "nanoseconds":
        raise ValueError("timing units must be explicit")
    names = [m.get("name") for m in metrics]
    if set(names) != EXPECTED or len(set(names)) != len(EXPECTED):
        raise ValueError("missing or duplicate operation workloads")
    for metric in metrics:
        if metric.get("kind") != "metric":
            raise ValueError("unexpected metric kind")
        name = metric["name"]
        expected = (
            1 if name == "reopen_replay" else
            run["records"] if name in {"put_sync", "delete_sync"} else
            run["operations"]
        )
        if type(metric.get("count")) is not int or metric["count"] != expected:
            raise ValueError(f"wrong sample count: {name}")
        values = [metric.get(k) for k in ("p50_ns", "p95_ns", "p99_ns", "max_ns")]
        if any(type(v) is not int or v < 0 for v in values):
            raise ValueError(f"negative or missing nanoseconds: {name}")
        if values != sorted(values):
            raise ValueError(f"unordered nearest-rank percentiles: {name}")
        for key in ("p50", "p95", "p99", "max"):
            milliseconds = metric.get(key + "_ms")
            if (not isinstance(milliseconds, (int, float))
                    or not math.isfinite(milliseconds) or milliseconds < 0):
                raise ValueError(f"invalid ms metric {name}/{key}")
            if abs(milliseconds - metric[key + "_ns"] / 1_000_000.0) > 0.0000011:
                raise ValueError(f"nanosecond/millisecond mismatch: {name}/{key}")
        for key in ("loop_wall_ms", "loop_wall_ops_per_sec"):
            value = metric.get(key)
            if not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
                raise ValueError(f"invalid measured wall/throughput: {name}/{key}")
    if (type(summary.get("disk_bytes")) is not int
            or summary["disk_bytes"] <= 0
            or type(summary.get("segment_count")) is not int
            or summary["segment_count"] < 1
            or not isinstance(summary.get("elapsed_ms"), (int, float))
            or not math.isfinite(summary["elapsed_ms"])
            or summary["elapsed_ms"] <= 0):
        raise ValueError("invalid durable disk/replay summary")
    return objects


def sample() -> str:
    run = {
        "schema": 1, "kind": "run", "engine": "rystemdb-storage",
        "engine_version": "0.1.2", "mode": "embedded_single_writer",
        "durability": "sync_data_per_acknowledged_write",
        "os": "linux", "arch": "x86_64", "seed": 42,
        "records": 4, "operations": 7, "payload_bytes": 128,
        "timing_unit": "nanoseconds",
    }
    measurements = []
    for name in sorted(EXPECTED):
        count = 1 if name == "reopen_replay" else 4 if name in {
            "put_sync", "delete_sync",
        } else 7
        measurements.append({
            "schema": 1, "kind": "metric", "name": name, "count": count,
            "p50_ns": 1_000, "p95_ns": 2_000, "p99_ns": 3_000,
            "max_ns": 4_000, "p50_ms": .001, "p95_ms": .002,
            "p99_ms": .003, "max_ms": .004,
            "loop_wall_ms": 2.0, "loop_wall_ops_per_sec": 500.0,
        })
    summary = {"schema": 1, "kind": "summary",
               "elapsed_ms": 25., "disk_bytes": 2048, "segment_count": 1}
    return "\n".join(json.dumps(item) for item in [run, *measurements, summary])


class ContractTests(unittest.TestCase):
    def test_valid_report(self) -> None:
        self.assertEqual(len(validate(sample())), 12)

    def test_missing_workload(self) -> None:
        rows = sample().splitlines()
        with self.assertRaises(ValueError):
            validate("\n".join(rows[:-2] + rows[-1:]))

    def test_percentiles_reversed(self) -> None:
        rows = [json.loads(s) for s in sample().splitlines()]
        rows[1]["p99_ns"] = 10
        with self.assertRaises(ValueError):
            validate("\n".join(json.dumps(x) for x in rows))

    def test_invalid_sample_count(self) -> None:
        rows = [json.loads(s) for s in sample().splitlines()]
        rows[1]["count"] += 1
        with self.assertRaises(ValueError):
            validate("\n".join(json.dumps(x) for x in rows))

    def test_not_finite_measurement(self) -> None:
        rows = [json.loads(s) for s in sample().splitlines()]
        rows[1]["loop_wall_ops_per_sec"] = float("nan")
        with self.assertRaises(ValueError):
            validate("\n".join(json.dumps(x) for x in rows))

    def test_missing_durable_footprint(self) -> None:
        rows = [json.loads(s) for s in sample().splitlines()]
        rows[-1]["disk_bytes"] = 0
        with self.assertRaises(ValueError):
            validate("\n".join(json.dumps(x) for x in rows))


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        unittest.main(argv=[sys.argv[0]], verbosity=2)
    elif len(sys.argv) == 2:
        lines = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
        objects = validate(lines)
        print(f"storage benchmark NDJSON validated: {len(objects) - 2} workloads")
    else:
        raise SystemExit("usage: check_storage_benchmark.py [--self-test | path.ndjson]")
