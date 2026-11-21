# ADR 0006 — RSWP binary framing, four SDK projects and distributed roadmap

Status: **Accepted for the frame contract; transport and distributed details are proposals** — 2026-11-21

## Context
Rystem Database needs non-JSON high-throughput point RPC, first-class clients for .NET, Rust and TypeScript, an EF Core provider, and eventual sharding and geo-redundancy. The actual project today is experimental single-node binary document storage with a local CLI.

## Decisions
1. Create `src/libraries` with C# SDK, Rust SDK, TypeScript SDK and a separate EF Core provider project.
2. Centralize binary wire framing in `src/rystemdb-protocol`: RSWP v1, 24-byte fixed header, BE integers, max 16MiB body, strict invalid-frame rejection, request ID and operation.
3. Byte-level interoperability tests must pass in Rust, .NET and Node/TypeScript before framing is usable in a server.
4. Prefer an eventual authenticated TLS transport. Evaluate TCP, QUIC and gRPC on realistic workloads. Arrow Flight SQL is a separate path for analytical Arrow batches, not an automatic replacement for point RPC.
5. Treat replicated WAL, sharding, cross-region failover, consistency and query optimizations as **distinct design/test gates**. Raft-style quorum consensus is the starting candidate, not a homegrown consensus algorithm.
6. Avoid public endpoints until security and durability gates are met. Incomplete SDK scaffolds must say so; no fake `DbContext` provider API.

## Deliberately deferred
- A working network server, TLS authentication, full CRUD body schema and SDK connections.
- EF query translation, Migrations, tracking and official provider specification compliance.
- Cluster metadata, sharding, replica synchronization, geo consistency, WAL coordination and distributed transactions.
- Claims of \"fastest protocol\" or novel storage superiority without reproducible benchmark evidence.

## Supporting references
- Apache Arrow Flight RPC and Flight SQL specifications for streaming analytical results.
- EF Core official database provider and writing-a-provider guidance for compatibility/specification tests.

## Verification
Issue #24 carries Rust RED/GREEN octet-level framing tests. Issues #27/#28 carry additional .NET/TypeScript RED/GREEN contracts. CI and exact commit links are recorded on those issues.
