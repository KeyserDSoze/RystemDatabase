//! Research-only, native Rust SQLite WAL/FULL benchmark for RystemDatabase.
//! THIS IS NOT A PRODUCTION SQLITE BACKEND OR FAIR ENGINE RANKING.
//! SQLite BLOB retrieval decodes typed RDB v1 but DOES NOT validate an RYSF
//! outer frame CRC. The corpus/order are parsed and checked against #92's
//! exact seeded Rust-generated NDJSON fixture before any measurements.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use rystemdb_core::{decode, encode, Value};
use rystemdb_storage::performance::{
    ns_to_ms, operations_per_second, seeded_indexes, summarize, synthetic_document,
};
use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const MAX_FIXTURE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 256;
const MAX_OPERATIONS: usize = 512;
const MAX_PAYLOAD_BYTES: usize = 1024;
const METRIC_NAMES: [&str; 7] = [
    "put_sync", "get_hit_typed_rdb_warm", "get_miss",
    "update_sync", "batch_2_sync", "delete_sync", "reopen_replay",
];

fn invalid(reason: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason.into())
}
fn numeric(row: &Json, name: &str) -> Result<u64> {
    row.get(name).and_then(Json::as_u64)
        .ok_or_else(|| invalid(format!("missing/invalid unsigned {name}")).into())
}
fn string<'a>(row: &'a Json, name: &str) -> Result<&'a str> {
    row.get(name).and_then(Json::as_str)
        .ok_or_else(|| invalid(format!("missing/invalid string {name}")).into())
}
fn shape(row: &Json, kind: &str, fields: &[&str]) -> Result<()> {
    let map = row.as_object().ok_or_else(|| invalid("fixture row is not an object"))?;
    if map.len() != fields.len() || fields.iter().any(|f| !map.contains_key(*f))
        || numeric(row, "schema")? != 1 || string(row, "kind")? != kind
    {
        return Err(invalid(format!("invalid {kind} row shape/schema")).into());
    }
    Ok(())
}
fn id(row: &Json, field: &str) -> Result<String> {
    let text = string(row, field)?;
    if text.is_empty() || text.len() > 1024 || text.as_bytes().contains(&0) {
        return Err(invalid("empty, oversized or NUL document id").into());
    }
    Ok(text.to_owned())
}
fn decode_hex(src: &str, expected: &Value) -> Result<Vec<u8>> {
    if src.len() < 12 || src.len() > 131_072 || !src.len().is_multiple_of(2) {
        return Err(invalid("invalid RDB hex length").into());
    }
    let mut result = Vec::with_capacity(src.len() / 2);
    for pair in src.as_bytes().chunks_exact(2) {
        let digits = std::str::from_utf8(pair)?;
        result.push(u8::from_str_radix(digits, 16)?);
    }
    // Reject checksum/typing bypass by requiring FULL canonical RDB v1
    // decoding and byte-for-byte equality to the seeded synthetic value.
    let typed = decode(&result)?;
    if &typed != expected || encode(expected)? != result {
        return Err(invalid("fixture value differs from the seeded canonical RDB document").into());
    }
    Ok(result)
}
fn record(row: &Json, expected_id: String, expected_value: Value) -> Result<Doc> {
    let actual_id = id(row, "id")?;
    if actual_id != expected_id {
        return Err(invalid(format!("unexpected, duplicate or out-of-order ID {actual_id}")).into());
    }
    let bytes = decode_hex(string(row, "value_hex")?, &expected_value)?;
    Ok(Doc { id: actual_id, bytes, typed: expected_value })
}

/// Document with both exact canonical RDB v1 bytes and the typed value.
/// Each normal timed SQLite read returns and decodes the typed document.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub id: String,
    pub bytes: Vec<u8>,
    pub typed: Value,
}

/// The very same corpus and key order exported by #92's actual Rystem
/// release benchmark. This is validated against independent seeded Rust
/// generators, not merely parsed from untrusted JSON with arbitrary IDs.
#[derive(Clone, Debug)]
pub struct Fixture {
    pub seed: u64,
    pub records: usize,
    pub operations: usize,
    pub payload_bytes: usize,
    pub initial: Vec<Doc>,
    pub read_ids: Vec<String>,
    pub updates: Vec<Doc>,
    pub batches: Vec<(Doc, Doc)>,
}

/// Reject unmatched fixtures *before* opening the timed benchmark database.
/// Exact RDB values must equal the seed-derived typed/null/bytes/ref corpus.
pub fn load_fixture(path: &Path) -> Result<Fixture> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_FIXTURE_BYTES {
        return Err(invalid("fixture input exceeds 16 MiB").into());
    }
    let input = fs::read_to_string(path)?;
    let rows = input.lines().filter(|s| !s.trim().is_empty())
        .map(serde_json::from_str::<Json>).collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err(invalid("empty RDB fixture").into());
    }
    let header = &rows[0];
    shape(header, "fixture_run", &[
        "schema", "kind", "seed", "records", "operations",
        "payload_bytes", "engine", "value_encoding",
    ])?;
    if string(header, "engine")? != "rystemdb-storage"
        || string(header, "value_encoding")? != "rdb_v1_frame_hex"
    {
        return Err(invalid("not a native Rystem typed RDB v1 fixture").into());
    }
    let seed = numeric(header, "seed")?;
    let records = usize::try_from(numeric(header, "records")?)?;
    let operations = usize::try_from(numeric(header, "operations")?)?;
    let payload_bytes = usize::try_from(numeric(header, "payload_bytes")?)?;
    if !(1..=MAX_RECORDS).contains(&records)
        || !(1..=MAX_OPERATIONS).contains(&operations)
        || payload_bytes > MAX_PAYLOAD_BYTES
    {
        return Err(invalid("fixture cardinality or payload exceeds supported bounded limits").into());
    }
    if rows.len() != 1 + records + operations * 3 {
        return Err(invalid("fixture row count does not match the source workload").into());
    }

    let mut pos = 1;
    let mut initial = Vec::with_capacity(records);
    for index in 0..records {
        let row = &rows[pos]; pos += 1;
        shape(row, "initial", &["schema", "kind", "id", "value_hex"])?;
        initial.push(record(
            row, format!("bench/{index:08}"), synthetic_document(seed, index, payload_bytes),
        )?);
    }

    let order = seeded_indexes(seed ^ 0xcafe_d00d, operations, records);
    let mut read_ids = Vec::with_capacity(operations);
    for (sequence, &index) in order.iter().enumerate() {
        let row = &rows[pos]; pos += 1;
        shape(row, "read", &["schema", "kind", "sequence", "id"])?;
        if numeric(row, "sequence")? != sequence as u64 {
            return Err(invalid("non-contiguous read operation sequence").into());
        }
        let actual = id(row, "id")?;
        if actual != format!("bench/{index:08}") {
            return Err(invalid("read sequence does not match the seeded Rystem workload").into());
        }
        read_ids.push(actual);
    }

    let mut updates = Vec::with_capacity(operations);
    for (sequence, &index) in order.iter().enumerate() {
        let row = &rows[pos]; pos += 1;
        shape(row, "update", &["schema", "kind", "sequence", "id", "value_hex"])?;
        if numeric(row, "sequence")? != sequence as u64 {
            return Err(invalid("non-contiguous update operation sequence").into());
        }
        updates.push(record(
            row, format!("bench/{index:08}"),
            synthetic_document(seed ^ 0xdeb6_5d53, records + sequence, payload_bytes),
        )?);
    }

    let mut batches = Vec::with_capacity(operations);
    for sequence in 0..operations {
        let row = &rows[pos]; pos += 1;
        shape(row, "batch", &[
            "schema", "kind", "sequence", "id_a", "value_a_hex",
            "id_b", "value_b_hex",
        ])?;
        if numeric(row, "sequence")? != sequence as u64 {
            return Err(invalid("non-contiguous batch operation sequence").into());
        }
        let expected_a = format!("batch/{sequence:08}/a");
        let expected_b = format!("batch/{sequence:08}/b");
        if id(row, "id_a")? != expected_a || id(row, "id_b")? != expected_b {
            return Err(invalid("unexpected keys in two-key atomic batch").into());
        }
        batches.push((
            Doc {
                id: expected_a,
                bytes: decode_hex(
                    string(row, "value_a_hex")?, &Value::Integer(sequence as i64),
                )?,
                typed: Value::Integer(sequence as i64),
            },
            Doc {
                id: expected_b,
                bytes: decode_hex(string(row, "value_b_hex")?, &Value::Null)?,
                typed: Value::Null,
            },
        ));
    }
    Ok(Fixture { seed, records, operations, payload_bytes, initial, read_ids, updates, batches })
}

/// Native SQLite connection in WAL + FULL mode, with explicit per-write
/// commits. SQLite's BLOB does NOT have the extra RYSF frame checksum.
/// Prepared read statements are cached by rusqlite inside this connection.
pub struct NativeSqlite {
    conn: Connection,
    journal_mode: String,
    synchronous: i64,
}
impl NativeSqlite {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; \
             CREATE TABLE IF NOT EXISTS docs (id TEXT PRIMARY KEY NOT NULL, \
             payload BLOB NOT NULL) WITHOUT ROWID;",
        )?;
        let journal_mode: String =
            conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        let synchronous: i64 =
            conn.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
        if journal_mode.to_ascii_lowercase() != "wal" || synchronous != 2 {
            return Err(invalid("SQLite WAL/FULL durability was not applied").into());
        }
        Ok(Self { conn, journal_mode, synchronous })
    }
    pub fn journal_mode(&self) -> &str { &self.journal_mode }
    pub fn synchronous(&self) -> i64 { self.synchronous }

    /// Returns (exact SQLite BLOB bytes, decoded typed RDB v1 Value).
    /// Decoding is done DURING the timed API call in the benchmark runner.
    pub fn get_typed(&self, id: &str) -> Result<Option<(Vec<u8>, Value)>> {
        let mut stmt = self.conn.prepare_cached("SELECT payload FROM docs WHERE id=?1")?;
        let raw: Option<Vec<u8>> =
            stmt.query_row(params![id], |row| row.get(0)).optional()?;
        match raw {
            Some(raw) => {
                let typed = decode(&raw)?;
                Ok(Some((raw, typed)))
            }
            None => Ok(None),
        }
    }
    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM docs", [], |row| row.get(0))?)
    }

    /// One acknowledged INSERT = one explicit WAL+FULL transaction.
    pub fn put(&mut self, id: &str, bytes: &[u8]) -> Result<usize> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "INSERT INTO docs (id,payload) VALUES (?1,?2)", params![id, bytes],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    /// One acknowledged update = one explicit WAL+FULL transaction.
    pub fn update(&mut self, id: &str, bytes: &[u8]) -> Result<usize> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE docs SET payload=?1 WHERE id=?2", params![bytes, id],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    pub fn delete(&mut self, id: &str) -> Result<usize> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute("DELETE FROM docs WHERE id=?1", params![id])?;
        tx.commit()?;
        Ok(changed)
    }

    /// Atomic 2-row insert, exactly one COMMIT, rollback on any failure.
    pub fn put_pair(&mut self, a: &str, bytes_a: &[u8], b: &str, bytes_b: &[u8]) -> Result<usize> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let first = tx.execute(
            "INSERT INTO docs (id,payload) VALUES (?1,?2)", params![a, bytes_a],
        )?;
        let second = tx.execute(
            "INSERT INTO docs (id,payload) VALUES (?1,?2)", params![b, bytes_b],
        )?;
        tx.commit()?;
        Ok(first + second)
    }

    /// Test-only corruption injection: SQLite can store malformed RDB bytes
    /// as opaque BLOB. Native benchmark typed GET MUST reject that value.
    pub fn overwrite_raw_for_corruption_test(&mut self, id: &str, raw: &[u8]) -> Result<()> {
        if self.update(id, raw)? != 1 {
            return Err(invalid("corruption test key missing").into());
        }
        Ok(())
    }
}

fn measure<T>(
    name: &'static str,
    count: usize,
    mut action: impl FnMut(usize) -> Result<T>,
    mut check: impl FnMut(usize, &T) -> bool,
) -> Result<Json> {
    if count == 0 { return Err(invalid("empty measured workload").into()); }
    let mut samples = Vec::with_capacity(count);
    let loop_wall = Instant::now();
    for i in 0..count {
        let start = Instant::now();
        let result = action(i)?;
        samples.push(start.elapsed().as_nanos().min(u64::MAX as u128) as u64);
        if !check(i, &result) {
            return Err(invalid(format!("{name} returned incorrect value at operation {i}")).into());
        }
    }
    let elapsed = loop_wall.elapsed();
    let stats = summarize(&samples).ok_or_else(|| invalid("no latency samples"))?;
    Ok(json!({
        "schema":1, "kind":"metric", "name":name, "count":stats.count,
        "p50_ns":stats.p50_ns, "p95_ns":stats.p95_ns,
        "p99_ns":stats.p99_ns, "max_ns":stats.max_ns,
        "p50_ms":round6(ns_to_ms(stats.p50_ns)),
        "p95_ms":round6(ns_to_ms(stats.p95_ns)),
        "p99_ms":round6(ns_to_ms(stats.p99_ns)),
        "max_ms":round6(ns_to_ms(stats.max_ns)),
        "loop_wall_ms":round6(elapsed.as_secs_f64()*1000.0),
        "loop_wall_ops_per_sec":round3(operations_per_second(count,elapsed)),
    }))
}
fn round6(n:f64)->f64 { (n*1_000_000.0).round()/1_000_000.0 }
fn round3(n:f64)->f64 { (n*1000.0).round()/1000.0 }

fn disk_footprint(path: &Path) -> Result<(u64, usize)> {
    let mut total = 0;
    let mut files = 0;
    for suffix in ["", "-wal", "-shm"] {
        let item = std::path::PathBuf::from(format!("{}{}", path.display(), suffix));
        if let Ok(meta) = fs::metadata(&item) {
            total += meta.len();
            files += 1;
        }
    }
    Ok((total, files))
}

/// Measure a real native Rust SQLite driver against the **already-validated**
/// Rystem fixture. Every insert/update/delete includes WAL FULL commit; each
/// typed hit read includes actual RDB decoding. No RYSF outer CRC parity.
pub fn run_fixture(fixture: &Fixture, path: &Path) -> Result<Vec<Json>> {
    if path.exists() {
        return Err(invalid("benchmark requires an empty SQLite path").into());
    }
    let all_wall = Instant::now();
    let mut db = NativeSqlite::open(path)?;
    let mut metrics = Vec::with_capacity(METRIC_NAMES.len());
    metrics.push(measure("put_sync", fixture.records,
        |i| {
            let record = &fixture.initial[i];
            db.put(&record.id, &record.bytes)
        },
        |_, count| *count == 1,
    )?);
    for record in &fixture.initial {
        if db.get_typed(&record.id)? != Some((record.bytes.clone(), record.typed.clone())) {
            return Err(invalid("original corpus read parity failed").into());
        }
    }
    for record in fixture.initial.iter().take(64) {
        if db.get_typed(&record.id)?.is_none() {
            return Err(invalid("SQLite warmup failed").into());
        }
    }
    let indexes = seeded_indexes(
        fixture.seed ^ 0xcafe_d00d, fixture.operations, fixture.records,
    );
    metrics.push(measure("get_hit_typed_rdb_warm", fixture.operations,
        |i| db.get_typed(&fixture.read_ids[i]),
        |i, result| {
            let record = &fixture.initial[indexes[i]];
            result.as_ref() == Some(&(record.bytes.clone(), record.typed.clone()))
        },
    )?);
    metrics.push(measure("get_miss", fixture.operations,
        |i| db.get_typed(&format!("never-present/{i:08}")),
        |_, result| result.is_none(),
    )?);
    metrics.push(measure("update_sync", fixture.operations,
        |i| {
            let record = &fixture.updates[i];
            db.update(&record.id, &record.bytes)
        },
        |_, count| *count == 1,
    )?);
    let mut current = BTreeMap::new();
    for record in &fixture.initial { current.insert(&record.id, record); }
    for record in &fixture.updates { current.insert(&record.id, record); }
    for (id, record) in current {
        if db.get_typed(id)? != Some((record.bytes.clone(), record.typed.clone())) {
            return Err(invalid("updated corpus parity mismatch").into());
        }
    }
    metrics.push(measure("batch_2_sync", fixture.operations,
        |i| {
            let (a,b) = &fixture.batches[i];
            db.put_pair(&a.id, &a.bytes, &b.id, &b.bytes)
        },
        |_, count| *count == 2,
    )?);
    for (a,b) in &fixture.batches {
        if db.get_typed(&a.id)? != Some((a.bytes.clone(),a.typed.clone()))
            || db.get_typed(&b.id)? != Some((b.bytes.clone(),b.typed.clone()))
        {
            return Err(invalid("two-key atomic insert parity failed").into());
        }
    }
    metrics.push(measure("delete_sync", fixture.records,
        |i| db.delete(&fixture.initial[i].id),
        |_, count| *count == 1,
    )?);
    if db.count()? != (fixture.operations * 2) as i64 {
        return Err(invalid("unexpected SQLite final row count after deletes").into());
    }
    for record in &fixture.initial {
        if db.get_typed(&record.id)?.is_some() {
            return Err(invalid("SQLite delete verification mismatch").into());
        }
    }
    let (disk_bytes, file_count) = disk_footprint(path)?;
    drop(db);
    let expected_first = &fixture.batches[0];
    metrics.push(measure("reopen_replay", 1,
        |_| NativeSqlite::open(path),
        |_, reopened| {
            reopened.count().ok() == Some((fixture.operations * 2) as i64)
                && reopened.get_typed(&expected_first.0.id).ok()
                    == Some(Some((expected_first.0.bytes.clone(),expected_first.0.typed.clone())))
                && reopened.get_typed(&expected_first.1.id).ok()
                    == Some(Some((expected_first.1.bytes.clone(),expected_first.1.typed.clone())))
                && reopened.get_typed(&fixture.initial[0].id).ok() == Some(None)
        },
    )?);

    let mut report = Vec::with_capacity(METRIC_NAMES.len() + 2);
    report.push(json!({
        "schema":1, "kind":"run", "engine":"sqlite",
        "engine_version":rusqlite::version(), "driver":"native_rust_rusqlite",
        "driver_version":"0.37.0", "mode":"embedded_single_writer",
        "sqlite_journal_mode":"wal","sqlite_synchronous":"FULL",
        "durability":"WAL_FULL_explicit_commit_each_write",
        "payload_semantics":"rdb_v1_typed_decode_no_rysf_outer_crc",
        "read_cache":"warmed","timing_unit":"nanoseconds",
        "seed":fixture.seed,"records":fixture.records,
        "operations":fixture.operations,"payload_bytes":fixture.payload_bytes,
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
    }));
    report.extend(metrics);
    report.push(json!({
        "schema":1,"kind":"summary","disk_bytes":disk_bytes,
        "sqlite_file_count":file_count,
        "elapsed_ms":round6(all_wall.elapsed().as_secs_f64()*1000.0),
    }));
    validate_report(&report, fixture)?;
    Ok(report)
}

/// An independent report-shape and statistical consistency gate, without
/// asserting benchmark latency thresholds from noisy shared CI runners.
pub fn validate_report(report: &[Json], fixture: &Fixture) -> Result<()> {
    if report.len() != METRIC_NAMES.len()+2 {
        return Err(invalid("wrong number of native SQLite report rows").into());
    }
    let run = &report[0];
    if string(run,"kind")? != "run" || numeric(run,"schema")? != 1
        || string(run,"engine")? != "sqlite"
        || string(run,"driver")? != "native_rust_rusqlite"
        || string(run,"sqlite_journal_mode")? != "wal"
        || string(run,"sqlite_synchronous")? != "FULL"
        || string(run,"payload_semantics")? != "rdb_v1_typed_decode_no_rysf_outer_crc"
        || numeric(run,"seed")? != fixture.seed
        || numeric(run,"records")? != fixture.records as u64
        || numeric(run,"operations")? != fixture.operations as u64
        || numeric(run,"payload_bytes")? != fixture.payload_bytes as u64
    {
        return Err(invalid("native SQLite run metadata mismatch").into());
    }
    let mut observed = BTreeSet::new();
    for row in &report[1..report.len()-1] {
        if string(row,"kind")? != "metric" || numeric(row,"schema")? != 1 {
            return Err(invalid("invalid native SQLite metric row").into());
        }
        let name = string(row,"name")?;
        let count = match name {
            "put_sync"|"delete_sync" => fixture.records,
            "reopen_replay" => 1,
            _ if METRIC_NAMES.contains(&name) => fixture.operations,
            _ => return Err(invalid("unknown native SQLite metric").into()),
        };
        if !observed.insert(name) || numeric(row,"count")? != count as u64 {
            return Err(invalid("duplicate metric or wrong sample count").into());
        }
        let names = ["p50","p95","p99","max"];
        let mut previous = 0;
        for label in names {
            let ns = numeric(row,&format!("{label}_ns"))?;
            let ms = row.get(format!("{label}_ms")).and_then(Json::as_f64)
                .ok_or_else(|| invalid("missing nonnegative millisecond metric"))?;
            if ns < previous || !ms.is_finite()
                || (ms - ns as f64 / 1_000_000.0).abs()>0.0000011
            {
                return Err(invalid("nonmonotonic or inconsistent latency percentiles").into());
            }
            previous = ns;
        }
        for name in ["loop_wall_ms","loop_wall_ops_per_sec"] {
            let value=row.get(name).and_then(Json::as_f64)
                .ok_or_else(|| invalid("missing wall-time/throughput"))?;
            if !value.is_finite() || value<0.0 {
                return Err(invalid("nonfinite wall-time/throughput").into());
            }
        }
    }
    if observed.len()!=METRIC_NAMES.len() {
        return Err(invalid("missing native SQLite workload").into());
    }
    let summary = &report[report.len()-1];
    if string(summary,"kind")? != "summary" || numeric(summary,"schema")? != 1
        || numeric(summary,"disk_bytes")?==0 || numeric(summary,"sqlite_file_count")?==0
    {
        return Err(invalid("missing SQLite main/WAL footprint").into());
    }
    Ok(())
}
