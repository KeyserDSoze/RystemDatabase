#!/usr/bin/env python3
"""Bounded alternating-order Rystem vs native Rust SQLite repeated benchmark.

Research / diagnostics only: GitHub-hosted CI is shared, not a dedicated
controlled machine. Report run-median and spread, NEVER a cross-engine winner.
All Rystem reads verify an outer RYSF CRC; SQLite typed reads do not.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
from typing import Any

from check_storage_benchmark import validate as validate_rystem
from check_native_sqlite_benchmark import check as validate_native_sqlite


# Scenario, Rystem operation, SQLite Rust operation, limitation.
SCENARIOS = (
    ("put_sync", "put_sync", "put_sync", "different_storage_and_journaling"),
    ("update_sync", "update_sync", "update_sync", "different_storage_and_journaling"),
    ("delete_sync", "delete_sync", "delete_sync", "different_storage_and_journaling"),
    ("typed_get_warm", "get_hit_typed_crc_warm", "get_hit_typed_rdb_warm",
     "non_equivalent_outer_rysf_crc"),
    ("get_miss", "get_miss", "get_miss", "non_equivalent_missing_lookup"),
    ("two_key_commit_sync", "staged_batch_2_sync", "batch_2_sync",
     "non_equivalent_staging_overhead"),
    ("reopen_replay", "reopen_replay", "reopen_replay",
     "non_equivalent_reopen_replay"),
)
MAX_REPETITIONS = 25
MAX_RECORDS = 256
MAX_OPERATIONS = 512
MAX_PAYLOAD_BYTES = 1024


def alternating_orders(repetitions: int, seed: int) -> list[tuple[str, str]]:
    """Balanced A/B ordering, deterministic given the experiment seed.

    Adjacent repetitions reverse order. Odd counts differ by at most one
    first position. This reduces fixed order bias but cannot remove OS noise.
    """
    if type(repetitions) is not int or not 5 <= repetitions <= MAX_REPETITIONS:
        raise ValueError("repetitions must be an integer between 5 and 25")
    if type(seed) is not int or not 0 <= seed < 2 ** 64:
        raise ValueError("seed must be an unsigned 64-bit integer")
    result = []
    for index in range(repetitions):
        result.append(
            ("rystem", "sqlite") if (index + seed) % 2 == 0
            else ("sqlite", "rystem")
        )
    return result


def verify_pair(rystem: list[dict], sqlite: list[dict]) -> None:
    """Validate each report fully, then reject any corpus or host drift."""
    try:
        validate_rystem("\n".join(json.dumps(row, allow_nan=False) for row in rystem))
        validate_native_sqlite(sqlite, rystem[0])
    except (TypeError, KeyError, AssertionError) as error:
        raise ValueError("invalid native benchmark report schema") from error

    rust_run, native_run = rystem[0], sqlite[0]
    if rust_run.get("engine") != "rystemdb-storage":
        raise ValueError("expected embedded Rystem Rust report")
    for attr in ("seed", "records", "operations", "payload_bytes"):
        if rust_run.get(attr) != native_run.get(attr):
            raise ValueError(f"Rystem/SQLite fixture mismatch: {attr}")
    for attr in ("os", "arch"):
        left, right = rust_run.get(attr), native_run.get(attr)
        if left is not None and right is not None and left != right:
            raise ValueError(f"Rystem/SQLite host mismatch: {attr}")
    if native_run.get("payload_semantics") != "rdb_v1_typed_decode_no_rysf_outer_crc":
        raise ValueError("native SQLite outer CRC integrity must not be overstated")
    if rust_run.get("read_cache") != "warmed" or native_run.get("read_cache") != "warmed":
        raise ValueError("benchmark is strictly warm-cache, not a cold-cache claim")


def aggregate_runs(pairs: list[tuple[list[dict], list[dict]]]) -> list[dict]:
    """Median of run-level percentiles, NOT pooled-sample percentiles.

    Each repetition independently initializes a fresh database and replays
    the same deterministic fixture. Min/max describe across-run variation,
    not confidence intervals or statistically independent page-cache state.
    """
    if not 5 <= len(pairs) <= MAX_REPETITIONS:
        raise ValueError("need at least five and at most 25 paired runs")
    expected_metadata = None
    for rystem, sqlite in pairs:
        verify_pair(rystem, sqlite)
        meta = (
            rystem[0]["seed"], rystem[0]["records"], rystem[0]["operations"],
            rystem[0]["payload_bytes"], rystem[0].get("engine_version"),
            sqlite[0].get("engine_version"),
            rystem[0].get("os"), rystem[0].get("arch"),
            sqlite[0].get("os"), sqlite[0].get("arch"),
        )
        if expected_metadata is None:
            expected_metadata = meta
        elif meta != expected_metadata:
            raise ValueError("inconsistent seed, corpus, engine versions or host across runs")

    output: list[dict] = []
    for scenario, rystem_name, sqlite_name, semantics in SCENARIOS:
        for engine, name, position in (
            ("rystem", rystem_name, 0), ("sqlite", sqlite_name, 1),
        ):
            run_p50: list[int] = []
            run_p95: list[int] = []
            run_p99: list[int] = []
            for pair in pairs:
                rows = {m["name"]: m for m in pair[position][1:-1]}
                if name not in rows:
                    raise ValueError(f"missing metric {name} for {engine}")
                metric = rows[name]
                run_p50.append(metric["p50_ns"])
                run_p95.append(metric["p95_ns"])
                run_p99.append(metric["p99_ns"])
            output.append({
                "schema": 1,
                "kind": "aggregate",
                "scenario": scenario,
                "engine": engine,
                "source_metric": name,
                "runs": len(pairs),
                "semantics": semantics,
                "median_run_p50_ns": statistics.median(run_p50),
                "min_run_p50_ns": min(run_p50),
                "max_run_p50_ns": max(run_p50),
                "median_run_p95_ns": statistics.median(run_p95),
                "min_run_p95_ns": min(run_p95),
                "max_run_p95_ns": max(run_p95),
                "median_run_p99_ns": statistics.median(run_p99),
                "min_run_p99_ns": min(run_p99),
                "max_run_p99_ns": max(run_p99),
                "median_run_p50_ms": statistics.median(run_p50) / 1_000_000,
                "median_run_p95_ms": statistics.median(run_p95) / 1_000_000,
                "median_run_p99_ms": statistics.median(run_p99) / 1_000_000,
            })
    return output


def _read_ndjson(path: Path) -> list[dict]:
    if not path.is_file() or path.stat().st_size > 2_000_000:
        raise ValueError(f"missing, empty or oversized raw benchmark: {path}")
    try:
        rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
                if line.strip()]
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid raw NDJSON in {path}") from error
    if not rows or any(type(row) is not dict for row in rows):
        raise ValueError(f"missing or invalid benchmark rows in {path}")
    return rows


def _run_binary(binary: Path, options: list[str]) -> None:
    """Run exactly once. An error never retries or becomes a valid report."""
    try:
        result = subprocess.run(
            [str(binary), *options],
            capture_output=True, text=True, check=False, timeout=240,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"benchmark process failed: {binary.name}") from error
    if result.returncode != 0:
        raise RuntimeError(
            f"{binary.name} exited {result.returncode}: {result.stderr[-1500:]}"
        )


def run_comparison(
    rystem_bin: Path,
    sqlite_bin: Path,
    out_dir: Path,
    repetitions: int,
    seed: int,
    records: int,
    operations: int,
    payload_bytes: int,
) -> list[dict]:
    """Perform independent fresh-database runs and archive every source report.

    The fixture is generated once by a preliminary, UNCOUNTED Rystem run,
    then every repeated Rystem and SQLite invocation uses that exact seed,
    size and key schedule. Each binary itself creates a new storage root.
    """
    schedule = alternating_orders(repetitions, seed)
    if (any(type(x) is not int for x in (records, operations, payload_bytes))
            or not 1 <= records <= MAX_RECORDS
            or not 1 <= operations <= MAX_OPERATIONS
            or not 0 <= payload_bytes <= MAX_PAYLOAD_BYTES):
        raise ValueError("invalid fixture size; records<=256, operations<=512, payload<=1024")
    out_dir = Path(out_dir)
    if out_dir.exists() and (not out_dir.is_dir() or any(out_dir.iterdir())):
        raise ValueError("benchmark output directory must be new or empty")
    rystem_bin = Path(rystem_bin).resolve()
    sqlite_bin = Path(sqlite_bin).resolve()
    for binary in (rystem_bin, sqlite_bin):
        if not binary.is_file():
            raise FileNotFoundError(f"prebuilt release benchmark binary missing: {binary}")
    out_dir.mkdir(parents=True, exist_ok=True)

    fixture = out_dir / "fixture.ndjson"
    preflight = out_dir / "fixture-generation-rystem.ndjson"
    common = [
        "--records", str(records), "--operations", str(operations),
        "--payload-bytes", str(payload_bytes), "--seed", str(seed),
    ]
    # Fixture creation has its own full Rystem run and is NEVER included
    # in the measured repetitions or published aggregate medians.
    _run_binary(rystem_bin, [*common, "--json-output", str(preflight),
                             "--fixture-output", str(fixture)])
    validate_rystem(preflight.read_text(encoding="utf-8"))
    if not fixture.is_file() or not 0 < fixture.stat().st_size <= 16_000_000:
        raise ValueError("Rystem preflight did not produce bounded RDB fixture")
    try:
        header = json.loads(fixture.open(encoding="utf-8").readline())
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid Rystem fixture header") from error
    if (header.get("kind") != "fixture_run"
            or any(header.get(k) != v for k, v in (
                ("seed", seed), ("records", records),
                ("operations", operations), ("payload_bytes", payload_bytes)
            ))):
        raise ValueError("exported RDB fixture metadata mismatch")
    fixture_sha256 = hashlib.sha256(fixture.read_bytes()).hexdigest()

    pairs = []
    trials = []
    for rep, order in enumerate(schedule):
        report_paths = {
            "rystem": out_dir / f"rystem-{rep:02d}.ndjson",
            "sqlite": out_dir / f"sqlite-native-{rep:02d}.ndjson",
        }
        for engine in order:
            output = report_paths[engine]
            if engine == "rystem":
                _run_binary(rystem_bin, [*common, "--json-output", str(output)])
            else:
                _run_binary(sqlite_bin, [
                    "--fixture", str(fixture), "--output", str(output),
                ])
            if hashlib.sha256(fixture.read_bytes()).hexdigest() != fixture_sha256:
                raise ValueError("fixture RDB bytes were modified during paired runs")
        rystem = _read_ndjson(report_paths["rystem"])
        sqlite = _read_ndjson(report_paths["sqlite"])
        verify_pair(rystem, sqlite)
        pairs.append((rystem, sqlite))
        trials.append({
            "schema": 1, "kind": "trial", "index": rep,
            "first": order[0], "second": order[1],
            "rystem_file": report_paths["rystem"].name,
            "sqlite_file": report_paths["sqlite"].name,
            "fixture_sha256": fixture_sha256,
        })

    aggregates = aggregate_runs(pairs)
    first_run = pairs[0]
    uname = platform.uname()
    metadata = {
        "schema": 1, "kind": "experiment",
        "rystem_engine_version": first_run[0][0]["engine_version"],
        "sqlite_engine_version": first_run[1][0]["engine_version"],
        "sqlite_driver": "native_rust_rusqlite",
        "source_sha": os.environ.get("GITHUB_SHA", "local_unverified"),
        "fixture_sha256": fixture_sha256,
        "seed": seed, "records": records, "operations": operations,
        "payload_bytes": payload_bytes, "repetitions": repetitions,
        "schedule": [list(order) for order in schedule],
        "os": uname.system, "kernel": uname.release,
        "arch": uname.machine, "cpu": uname.processor or "unknown",
        "logical_cpu_count": os.cpu_count(),
        "shared_ci_runner": bool(os.environ.get("GITHUB_ACTIONS")),
        "cache": "warm_page_cache_not_cold_controlled",
        "host_control": "not_dedicated_or_cpu_pinned",
        "write_durability": "Rystem_sync_per_write_vs_SQLite_WAL_FULL",
        "get_integrity": "Rystem_full_RYSF_crc_and_typed_vs_SQLite_typed_without_outer_crc",
        "interpretation": "diagnostic_run_medians_not_world_speed_ranking",
        "preflight": preflight.name,
    }
    lines = [metadata, *trials, *aggregates]
    # Publish only AFTER all measurements, run-level correctness and
    # aggregate checks succeed. Failed runs may leave raw diagnostic files
    # but can never leave a success-looking final summary.
    (out_dir / "summary.ndjson").write_text(
        "\n".join(json.dumps(row, allow_nan=False, separators=(",", ":"))
                  for row in lines) + "\n", encoding="utf-8",
    )
    return lines


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rystem-bin", required=True, type=Path)
    parser.add_argument("--sqlite-bin", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--records", type=int, default=32)
    parser.add_argument("--operations", type=int, default=64)
    parser.add_argument("--payload-bytes", type=int, default=64)
    args = parser.parse_args()
    rows = run_comparison(
        args.rystem_bin, args.sqlite_bin, args.out_dir, args.repetitions,
        args.seed, args.records, args.operations, args.payload_bytes,
    )
    for row in rows:
        print(json.dumps(row, separators=(",", ":"), allow_nan=False))


if __name__ == "__main__":
    main()
