# ADR 0005 — Native document CLI and explicit typed JSON

Status: Accepted (experimental single-node local CLI) — 2026-11-21

Linked issue: [#21](https://github.com/KeyserDSoze/RystemDatabase/issues/21)

## Context

The binary segment store and typed nested codec are tested but were only accessible to embedded Rust clients. Users need an installable executable that can persist real documents without a separate language runtime or network database server.

## Decision

- Wire `rystemdb-cli` to `rystemdb-storage` and use the existing typed `Value` representation on disk.
- Use `serde_json` strictly for the **I/O boundary**, not as a persistence format. Commit the resolved Cargo.lock so all platforms and Docker build with `--locked`.
- Accept `document put|get|delete`, with `--data`, `--collection`, `--id`; PUT reads `--json -` or a local file. Validate and parse before opening a store so invalid input does not mutate database files.
- Require object roots and signed i64 integers. Finite f64 values are supported; exact decimals and big integers are deferred (do not silently round huge positive integers).
- Define explicit `$rystem` tagged envelopes (version 1) for references and hex bytes. PUT only interprets these tags under explicit `--typed-json`; a reserved `type: object` wrapper disambiguates a literal map.
- Use separate CLI processes as TDD integration tests; compare stored native values through the embedded Rust API. Add Docker persistence smoke tests with a named volume.
- The network `serve` command remains health-only. No unrequested unauthenticated HTTP document API is exposed.

## Risks

- The typed JSON wire contract is preliminary and not a database file format; app clients need schema compatibility before broad use.
- JSON floating-point representation is approximate and not suitable for exact finance; decimal128 is a future data type.
- References are not foreign-key constraints yet; transactionally consistent relationships remain a separate issue.
- The CLI requires exclusive writer access even for GET; concurrent multi-client reads are future work.
- This does not implement indexing, MVCC, multi-document transactions, backup, or production recovery guarantees.

## Evidence

Initial RED CLI behavior tests committed on `main` in [c4ee602](https://github.com/KeyserDSoze/RystemDatabase/commit/c4ee6029cc6f560a235417edaed8673628bb7e7e); after wiring the storage crate in [33dc356](https://github.com/KeyserDSoze/RystemDatabase/commit/33dc35620ff45711c3c4adc629f59e63e8e6deb2), Linux CI recorded **six behavioral RED failures** (not a compilation failure). Implementation [43a2713](https://github.com/KeyserDSoze/RystemDatabase/commit/43a2713a847e46f2d87aa551b23ad73a18a243ea) and locked JSON parser dependency [cb58965](https://github.com/KeyserDSoze/RystemDatabase/commit/cb58965ed8aa40607fbe5e4ab589c731bd422a84).

The full final GREEN commit/CI/release artifacts are recorded on Issue #21; acceptance requires all jobs and externally published assets to be checked before closure.
