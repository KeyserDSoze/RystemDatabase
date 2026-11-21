# Distributed Rystem Database — design proposal, NOT implemented

Epic: [#23](https://github.com/KeyserDSoze/RystemDatabase/issues/23).

## Proposed layers

```text
C# / Rust / TypeScript SDK (EF Core on top of C#)
    |
    | RSWP binary document/point RPC over authenticated TLS
    | Arrow Flight SQL for analytics (future)
    v
Gateway / topology discovery / query coordinator
    |
    +-- shard A, placement epoch 10
    |     leader -- committed Raft log --> follower(s)
    +-- shard B, placement epoch 14
    |     leader -- committed Raft log --> follower(s)
    +-- shard C ...
           |
           +-- asynchronous encrypted cross-region replication
                      +-- regional standby / DR
```

### 1. Partitioning & request routing (#30)

- Use a deterministic typed canonical key hash (or chosen partition key) with stable shard IDs; keep placement separate from identity.
- Metadata/versioned placement map has a monotonically increasing **epoch**. Prevent stale routers from writing to a previous shard leader during migrations; reject or redirect with a newer epoch.
- Online rebalancing: manifest/snapshot transfer + ordered change log delta + ownership cutover barrier + retention until readers finish.
- Cross-shard references/joins and hot partitions require explicit routing and colocation/cost policies, not hidden full scatter scans. Cross-shard transactions are not yet assumed.

### 2. Multi-instance replication (#31)

- Use a proven replicated state machine and consensus implementation (e.g., Raft), **do not invent a new consensus protocol** for production data.
- Replicate a durable command log that applies atomic changes to document pointers, secondary index entries, reference edges and commit sequence.
- An acknowledged write must be committed according to the configured quorum; a leader crash cannot create a divergent acknowledged history.
- Define follower reads, read-your-writes, snapshot and linearizable read modes and their latency/consistency tradeoffs.
- Failure injection: network partition, lost quorum, old leader fencing, snapshot corruption, follower catch-up, rolling schema upgrade, restored backup.

### 3. Geo-redundancy (#32)

- Recommended initial model: one region with a synchronous write quorum, encrypted **asynchronous** replicas in one or more remote regions.
- Async geo replication has **nonzero possible data loss** in a regional catastrophe. Track replica lag, maximum RPO, expected RTO, cutoff for stale reads, and controlled promotion/fencing.
- Cross-region synchronous quorum is an optional policy with higher WAN write latency and lower write availability under partition. No implicit multi-master merges.
- Resync through validated snapshots followed by ordered WAL increments; require restore/disaster drills before claiming availability targets.

### 4. Physical data/query engine (#33/#34)

- Explore typed field-ID dictionaries, sparse nested binary values, varint/zig-zag integers, optional per-block compression, record offsets and CRC32C.
- Benchmark an indexed row/document store for point reads and a derived Arrow/Parquet projection for vectorized aggregations. Keep snapshot sequence overlay; never double-count modified/deleted rows.
- Compare LSM/B+ tree/ART or alternative postings with measured workloads; choose implementation by throughput, p99, disk and RAM, not marketing.
- Preserve crash-safe migration from the current self-describing RDB/RYSF v1 segments.

## Security / release gates

No public cluster port until TLS1.3, authenticated clients, authz, observability, log redaction, storage access controls, backup and restore, WAL/MVCC durability and enforced writer fencing are tested. All features proceed through RED/GREEN TDD directly on `main`; the first increment is the bounded RSWP frame codec, **not** clustering.

Associated issues: #23 (epic), #24 (protocol), #25 (server), #26–29 (SDK/EF), #30 (shards), #31 (multi-instance), #32 (geo), #33 (binary layout), #34 (index/query), #35 (Arrow Flight SQL).
