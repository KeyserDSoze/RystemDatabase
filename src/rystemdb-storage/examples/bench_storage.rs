//! Reproducible embedded single-writer storage latency benchmark (#88).
//! Run: cargo run --release --locked -p rystemdb-storage --example bench_storage --
//!     --records 128 --operations 256 --payload-bytes 128 --seed 42
//! This is NOT a fair cross-engine comparison or a timing-based unit test.
//! CI runners are shared, and measurements must be repeated on dedicated hosts.
use rystemdb_core::Value;
use rystemdb_storage::performance::{
    ns_to_ms, operations_per_second, seeded_indexes, summarize, synthetic_document,
};
use rystemdb_storage::{DocumentStore, StoreError};
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Debug)]
struct Config {
    records: usize,
    operations: usize,
    payload_bytes: usize,
    seed: u64,
    json_output: Option<PathBuf>,
}

fn input_error(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, message.into())
}

fn config() -> Result<Config> {
    let mut result = Config {
        records: 128,
        operations: 256,
        payload_bytes: 128,
        seed: 42,
        json_output: None,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or_else(|| input_error(format!("missing value for {flag}")))?;
        match flag.as_str() {
            "--records" => result.records = value.parse()?,
            "--operations" => result.operations = value.parse()?,
            "--payload-bytes" => result.payload_bytes = value.parse()?,
            "--seed" => result.seed = value.parse()?,
            "--json-output" => result.json_output = Some(PathBuf::from(value)),
            _ => return Err(input_error(format!("unknown option {flag}")).into()),
        }
    }
    if !(1..=8_192).contains(&result.records)
        || !(1..=4_096).contains(&result.operations)
        || result.payload_bytes > 8_192
    {
        return Err(input_error(
            "records must be 1..=8192, operations 1..=4096, payload bytes 0..=8192",
        ).into());
    }
    Ok(result)
}

struct TempRoot(PathBuf);
impl TempRoot {
    fn create(seed: u64) -> Result<Self> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rystem-bench-{}-{seed}-{nanos}", std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempRoot {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

struct Metric {
    name: &'static str,
    samples_ns: Vec<u64>,
    loop_wall: Duration,
}

/// Time only the API operation, then verify its result OUTSIDE the timed
/// sample. The reported loop-wall throughput still includes verification and
/// clock overhead and must not be confused with isolated API calls per second.
fn measure<T>(
    name: &'static str,
    count: usize,
    mut operation: impl FnMut(usize) -> std::result::Result<T, StoreError>,
    mut verify: impl FnMut(usize, &T) -> bool,
) -> Result<Metric> {
    let mut samples_ns = Vec::with_capacity(count);
    let wall = Instant::now();
    for index in 0..count {
        let begin = Instant::now();
        let value = operation(index)?;
        let elapsed = begin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        samples_ns.push(elapsed);
        if !verify(index, black_box(&value)) {
            return Err(io::Error::other(format!(
                "incorrect database result in {name}, sample {index}"
            )).into());
        }
    }
    Ok(Metric { name, samples_ns, loop_wall: wall.elapsed() })
}

fn disk_footprint(root: &Path) -> Result<(u64, usize)> {
    let mut bytes = 0_u64;
    let mut segments = 0_usize;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().ends_with(".rdb") {
            bytes = bytes.saturating_add(entry.metadata()?.len());
            segments += 1;
        }
    }
    Ok((bytes, segments))
}

fn run() -> Result<()> {
    let cfg = config()?;
    let root = TempRoot::create(cfg.seed)?;
    let all_wall = Instant::now();

    let keys = (0..cfg.records).map(|i| format!("bench/{i:08}")).collect::<Vec<_>>();
    let corpus = (0..cfg.records).map(|i| {
        synthetic_document(cfg.seed, i, cfg.payload_bytes)
    }).collect::<Vec<_>>();
    let indexes = seeded_indexes(cfg.seed ^ 0xcafe_d00d, cfg.operations, cfg.records);
    let misses = (0..cfg.operations)
        .map(|i| format!("never-present/{i:08}"))
        .collect::<Vec<_>>();
    let updates = (0..cfg.operations).map(|i| {
        synthetic_document(cfg.seed ^ 0xdeb6_5d53, cfg.records + i, cfg.payload_bytes)
    }).collect::<Vec<_>>();
    let batch_keys = (0..cfg.operations).map(|i| (
        format!("batch/{i:08}/a"), format!("batch/{i:08}/b"),
    )).collect::<Vec<_>>();

    let mut db = DocumentStore::open(root.path())?;
    let mut metrics = Vec::new();

    metrics.push(measure("put_sync", cfg.records,
        |i| db.put(&keys[i], &corpus[i]),
        |_, _| true,
    )?);
    // Full typed read, CRC and RDB decoder; warm the OS cache before timing.
    for i in 0..cfg.records.min(64) {
        if db.get(&keys[i])?.as_ref() != Some(&corpus[i]) {
            return Err(io::Error::other("warmup read mismatch").into());
        }
    }
    metrics.push(measure("get_hit_typed_crc_warm", cfg.operations,
        |i| db.get(&keys[indexes[i]]),
        |i, value| value.as_ref() == Some(&corpus[indexes[i]]),
    )?);
    metrics.push(measure("get_miss", cfg.operations,
        |i| db.get(&misses[i]),
        |_, value| value.is_none(),
    )?);
    metrics.push(measure("snapshot_capture", cfg.operations,
        |_| Ok(db.snapshot()),
        |_, view| view.len() == cfg.records,
    )?);

    metrics.push(measure("update_sync", cfg.operations,
        |i| db.put(&keys[indexes[i]], &updates[i]),
        |_, _| true,
    )?);
    let mut latest = corpus.clone();
    for i in 0..cfg.operations {
        latest[indexes[i]] = updates[i].clone();
    }
    for i in 0..cfg.records {
        if db.get(&keys[i])?.as_ref() != Some(&latest[i]) {
            return Err(io::Error::other("update verification mismatch").into());
        }
    }

    // Staging and commit are both timed; each iteration commits exactly
    // two new keys in one synced checksummed RYSF BATCH frame.
    metrics.push(measure("staged_batch_2_sync", cfg.operations,
        |i| {
            let mut draft = db.staged_batch();
            draft.put(&batch_keys[i].0, Value::Integer(i as i64))?;
            draft.put(&batch_keys[i].1, Value::Null)?;
            db.commit_staged_batch(&draft)
        },
        |_, _| true,
    )?);
    for i in 0..cfg.operations {
        if db.get(&batch_keys[i].0)? != Some(Value::Integer(i as i64))
            || db.get(&batch_keys[i].1)? != Some(Value::Null)
        {
            return Err(io::Error::other("atomic batch verification mismatch").into());
        }
    }

    metrics.push(measure("delete_sync", cfg.records,
        |i| db.delete(&keys[i]),
        |_, success| *success,
    )?);
    for key in &keys {
        if db.get(key)?.is_some() {
            return Err(io::Error::other("delete verification mismatch").into());
        }
    }
    drop(db);

    metrics.push(measure("reopen_replay", 1,
        |_| DocumentStore::open(root.path()),
        |_, store| {
            store.len() == cfg.operations * 2
                && store.get(&batch_keys[0].0).ok() == Some(Some(Value::Integer(0)))
                && store.get(&keys[0]).ok() == Some(None)
        },
    )?);
    let (disk_bytes, segment_count) = disk_footprint(root.path())?;

    // All output lines are valid NDJSON for streaming consumers and artifacts.
    // This schema is stable for validation; no competitor engine values
    // are fabricated and there are no absolute latency pass/fail thresholds.
    let mut output = Vec::new();
    output.push(format!(
        "{{\"schema\":1,\"kind\":\"run\",\"engine\":\"rystemdb-storage\",\"engine_version\":\"{}\",\"mode\":\"embedded_single_writer\",\"transport\":\"none\",\"durability\":\"sync_data_per_acknowledged_write\",\"os\":\"{}\",\"arch\":\"{}\",\"seed\":{},\"records\":{},\"operations\":{},\"payload_bytes\":{},\"read_cache\":\"warmed\",\"timing_unit\":\"nanoseconds\"}}",
        env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH,
        cfg.seed, cfg.records, cfg.operations, cfg.payload_bytes,
    ));
    for metric in &metrics {
        let stats = summarize(&metric.samples_ns)
            .ok_or_else(|| io::Error::other("empty metric"))?;
        output.push(format!(
            "{{\"schema\":1,\"kind\":\"metric\",\"name\":\"{}\",\"count\":{},\"p50_ns\":{},\"p95_ns\":{},\"p99_ns\":{},\"max_ns\":{},\"p50_ms\":{:.6},\"p95_ms\":{:.6},\"p99_ms\":{:.6},\"max_ms\":{:.6},\"loop_wall_ms\":{:.6},\"loop_wall_ops_per_sec\":{:.3}}}",
            metric.name, stats.count, stats.p50_ns, stats.p95_ns,
            stats.p99_ns, stats.max_ns, ns_to_ms(stats.p50_ns),
            ns_to_ms(stats.p95_ns), ns_to_ms(stats.p99_ns),
            ns_to_ms(stats.max_ns), metric.loop_wall.as_secs_f64() * 1_000.0,
            operations_per_second(stats.count, metric.loop_wall),
        ));
    }
    output.push(format!(
        "{{\"schema\":1,\"kind\":\"summary\",\"elapsed_ms\":{:.6},\"disk_bytes\":{},\"segment_count\":{}}}",
        all_wall.elapsed().as_secs_f64() * 1_000.0, disk_bytes, segment_count,
    ));
    let ndjson = output.join("\n") + "\n";
    print!("{ndjson}");
    if let Some(destination) = cfg.json_output {
        fs::write(destination, ndjson)?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("storage benchmark failed: {error}");
        std::process::exit(1);
    }
}
