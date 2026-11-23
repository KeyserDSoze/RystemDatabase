//! #88 RED: deterministic synthetic corpus and statistically correct
//! performance telemetry. No CI assertion on elapsed time: shared runners
//! are unsuitable for hard latency gates.
use rystemdb_core::{decode, encode, Value};
use rystemdb_storage::performance::{
    ns_to_ms, operations_per_second, seeded_indexes, summarize, synthetic_document,
};
use std::time::Duration;

#[test]
fn tdd_latency_nearest_rank_percentiles_and_empty_samples() {
    assert!(summarize(&[]).is_none());
    let one = summarize(&[7]).unwrap();
    assert_eq!(one.count, 1);
    assert_eq!((one.p50_ns, one.p95_ns, one.p99_ns, one.max_ns), (7,7,7,7));
    assert_eq!(one.total_ns, 7);
    let odd = summarize(&[90,20,80,30,10,60,40,50,70]).unwrap();
    assert_eq!(odd.count, 9);
    assert_eq!(odd.p50_ns, 50);
    assert_eq!(odd.p95_ns, 90);
    assert_eq!(odd.p99_ns, 90);
    assert_eq!(odd.max_ns, 90);
    assert_eq!(odd.total_ns, 450);
    let even = summarize(&[10,30,20,40]).unwrap();
    assert_eq!(even.p50_ns, 20, "nearest-rank p50 is rank 2, not interpolated");
    assert_eq!(even.p95_ns, 40);
    assert_eq!(even.p99_ns, 40);
    assert_eq!(even.total_ns, 100);
}

#[test]
fn tdd_latency_counts_and_throughput_avoid_overflow_and_zero_duration() {
    let result = summarize(&[u64::MAX, u64::MAX, 0]).unwrap();
    assert_eq!(result.total_ns, u128::from(u64::MAX) * 2);
    assert_eq!(result.p50_ns, u64::MAX);
    assert_eq!(result.max_ns, u64::MAX);
    assert_eq!(ns_to_ms(2_500_000), 2.5);
    assert_eq!(ns_to_ms(500_000), 0.5);
    assert_eq!(operations_per_second(1_000, Duration::from_secs(2)), 500.0);
    assert_eq!(operations_per_second(0, Duration::from_secs(2)), 0.0);
    assert_eq!(operations_per_second(1, Duration::ZERO), 0.0,
        "zero elapsed time cannot imply infinite throughput");
}

#[test]
fn tdd_seeded_access_sequence_is_repeatable_bounded_and_seed_sensitive() {
    let first = seeded_indexes(42, 256, 17);
    assert_eq!(first.len(), 256);
    assert!(first.iter().all(|&i| i < 17));
    assert_eq!(first, seeded_indexes(42, 256, 17));
    assert_ne!(first, seeded_indexes(43, 256, 17));
    assert_eq!(seeded_indexes(42, 10, 1), vec![0; 10]);
    assert!(seeded_indexes(42, 10, 0).is_empty(),
        "empty cardinality produces no invalid index");
}

#[test]
fn tdd_synthetic_nested_sparse_typed_fixture_has_canonical_rdb_round_trip() {
    let a = synthetic_document(7, 0, 128);
    let b = synthetic_document(7, 1, 128);
    let sparse = synthetic_document(7, 2, 128);
    assert_eq!(a, synthetic_document(7, 0, 128));
    assert_ne!(a, b);
    for doc in [a, b, sparse] {
        let bytes = encode(&doc).unwrap();
        assert_eq!(decode(&bytes).unwrap(), doc);
        let Value::Object(fields) = doc else { panic!("document must be an object"); };
        assert_eq!(fields.get("nullable"), Some(&Value::Null));
        assert!(matches!(fields.get("items"), Some(Value::Array(_))));
        assert!(matches!(fields.get("owner"), Some(Value::Reference{..})));
        assert!(matches!(fields.get("payload"), Some(Value::Bytes(_))));
        if let Some(Value::Bytes(bytes)) = fields.get("payload") {
            assert_eq!(bytes.len(), 128);
        }
    }
    assert_ne!(
        synthetic_document(7, 0, 128),
        synthetic_document(8, 0, 128)
    );
}
