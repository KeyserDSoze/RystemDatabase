use rystemdb_protocol::crud::{decode_key, encode_key, CrudError};

const ASCII: &[u8] = b"\x00\x05users\x00\x03u42";
const UNICODE: &[u8] = b"\x00\x06orders\x00\x04\xf0\x9f\xa6\x8a";
// RDB binary for { "n": 42 } (object root; 1 field; signed i64).
const DOC: &[u8] = b"RDB\0\x01\x08\x00\x00\x00\x01\x00\x00\x00\x01n\x03\x00\x00\x00\x00\x00\x00\x00\x2a";

#[test]
fn tdd_ascii_key_golden_matches_all_sdk_languages() {
    assert_eq!(encode_key("users", "u42").unwrap(), ASCII);
    let parsed = decode_key(ASCII).unwrap();
    assert_eq!((parsed.collection,parsed.id,parsed.document),("users","u42",&[][..]));
}
#[test]
fn tdd_unicode_key_and_binary_put_payload_are_exact() {
    assert_eq!(encode_key("orders", "🦊").unwrap(), UNICODE);
    let mut put = UNICODE.to_vec();
    put.extend_from_slice(DOC);
    let decoded = decode_key(&put).unwrap();
    assert_eq!(decoded.collection, "orders");
    assert_eq!(decoded.id, "🦊");
    assert_eq!(decoded.document, DOC);
    assert_eq!(rystemdb_core::decode(decoded.document).unwrap(),
        rystemdb_core::Value::Object(std::collections::BTreeMap::from([
            ("n".to_string(), rystemdb_core::Value::Integer(42))
        ])));
}
#[test]
fn tdd_invalid_collection_id_and_lengths_fail_closed() {
    for coll in ["", "bad/name", "a b", "é", &"x".repeat(129)] {
        assert!(matches!(encode_key(coll,"id"),Err(CrudError::InvalidCollection)));
    }
    for id in ["", "\n", "\u{0085}", &"x".repeat(1024)] {
        assert!(matches!(encode_key("users",id),Err(CrudError::InvalidId)));
    }
    assert_eq!(decode_key(&[0, 1, b'/', 0, 1, b'x']), Err(CrudError::InvalidCollection));
    assert_eq!(decode_key(&[0, 1, b'u', 0, 1, 0]), Err(CrudError::InvalidId));
}
#[test]
fn tdd_every_key_truncation_and_utf8_corruption_is_rejected() {
    for n in 0..ASCII.len() {
        assert!(decode_key(&ASCII[..n]).is_err(),"accepted truncated key length {n}");
    }
    let mut bad = ASCII.to_vec();
    bad[10] = 0xff;
    assert_eq!(decode_key(&bad), Err(CrudError::InvalidId));
    let mut bad = ASCII.to_vec();
    bad[1] = 129;
    assert_eq!(decode_key(&bad), Err(CrudError::InvalidCollection));
}
#[test]
fn tdd_wire_length_overflow_is_rejected_without_allocating() {
    assert!(matches!(decode_key(&[]),Err(CrudError::Truncated)));
    let huge_id = "x".repeat(1020); // 5 + "/" + 1020 is >1024
    assert_eq!(encode_key("users",&huge_id),Err(CrudError::InvalidId));
}
