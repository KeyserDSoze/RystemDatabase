//! Dependency-free, opt-in benchmark building blocks.
//! No clocks, telemetry or allocations are inserted into database hot paths.
//! These helpers do NOT establish competitor performance or stable CI time gates.
use rystemdb_core::Value;
use std::collections::BTreeMap;
use std::time::Duration;

/// Nearest-rank summary of measured operation latencies in **nanoseconds**.
/// For n samples, p95 is the sample at sorted rank ceil(n*95/100).
/// No interpolation, averaging of percentiles, or hidden warmup samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatencySummary {
    pub count: usize,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub total_ns: u128,
}

/// Compute exact nearest-rank percentiles from the caller's measured samples.
/// Sorts a copy; does not modify the input and handles u64::MAX without
/// integer overflow in total duration or percentile index arithmetic.
pub fn summarize(samples: &[u64]) -> Option<LatencySummary> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    fn rank_index(count: usize, percentile: usize) -> usize {
        let whole = (count / 100) * percentile;
        let tail = ((count % 100) * percentile).div_ceil(100);
        whole.saturating_add(tail).saturating_sub(1).min(count - 1)
    }
    let count = sorted.len();
    Some(LatencySummary {
        count,
        p50_ns: sorted[rank_index(count, 50)],
        p95_ns: sorted[rank_index(count, 95)],
        p99_ns: sorted[rank_index(count, 99)],
        max_ns: sorted[count - 1],
        total_ns: sorted.iter().map(|&n| u128::from(n)).sum(),
    })
}

/// Convert **nanoseconds** to decimal milliseconds. No rounding is applied.
pub fn ns_to_ms(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1_000_000.0
}

/// Throughput from full operation-loop wall time, not the sum of individually
/// sampled durations. A zero count/duration never claims infinite throughput.
pub fn operations_per_second(count: usize, elapsed: Duration) -> f64 {
    if count == 0 || elapsed.is_zero() {
        0.0
    } else {
        count as f64 / elapsed.as_secs_f64()
    }
}

#[derive(Clone, Copy)]
struct Prng(u64);
impl Prng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed })
    }

    // Simple deterministic xorshift* for workloads; NOT cryptography.
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Deterministic, repeatable pseudo-random point IDs in [0, cardinality).
/// Zero cardinality produces an empty list; callers must validate dataset
/// sizes before timing. This uses a single seed, not OS-generated randomness.
pub fn seeded_indexes(seed: u64, count: usize, cardinality: usize) -> Vec<usize> {
    if cardinality == 0 {
        return Vec::new();
    }
    let mut rng = Prng::new(seed);
    (0..count).map(|_| (rng.next() % cardinality as u64) as usize).collect()
}

/// One nested, sparse, typed RDB document for repeatable microbenchmarks.
/// Contains bytes, explicit NULL, a typed reference and an array. A subset
/// contains additional object fields, so field sparsity is deterministic.
/// `payload_bytes` is the length of the *binary field* (not full RDB frame).
pub fn synthetic_document(seed: u64, index: usize, payload_bytes: usize) -> Value {
    let mut rng = Prng::new(
        seed ^ (index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ 0xd1b5_4a32_d192_ed03,
    );
    let mut data = vec![0_u8; payload_bytes];
    for chunk in data.chunks_mut(8) {
        let word = rng.next().to_le_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
    let mut root = BTreeMap::new();
    root.insert("id".to_owned(), Value::Integer(index as i64));
    root.insert("nullable".to_owned(), Value::Null);
    root.insert("owner".to_owned(), Value::Reference {
        collection: "users".into(),
        id: format!("user/{:08}", rng.next() % 1_000),
    });
    root.insert("payload".to_owned(), Value::Bytes(data));
    root.insert("items".to_owned(), Value::Array(vec![
        Value::Integer((rng.next() % 100_000) as i64),
        Value::Bool(rng.next() & 1 == 1),
        Value::Null,
    ]));
    if index.is_multiple_of(3) {
        let mut nested = BTreeMap::new();
        nested.insert("region".to_owned(), Value::String("EU".into()));
        nested.insert("priority".to_owned(), Value::Integer((rng.next() % 7) as i64));
        root.insert("sparse".to_owned(), Value::Object(nested));
    }
    Value::Object(root)
}
