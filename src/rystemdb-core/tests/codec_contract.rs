use std::collections::BTreeMap;

use rystemdb_core::{decode, encode, CodecError, Value};

fn nested_document() -> Value {
    let mut nested = BTreeMap::new();
    nested.insert("city".into(), Value::String("Roma 🏛️".into()));
    nested.insert("nullable".into(), Value::Null);
    let mut root = BTreeMap::new();
    root.insert("shipping".into(), Value::Object(nested));
    root.insert("items".into(), Value::Array(vec![
        Value::Integer(42),
        Value::Reference { collection: "users".into(), id: "u/1".into() },
        Value::Bytes(vec![0, 1, 2, 255]),
    ]));
    root.insert("active".into(), Value::Bool(true));
    Value::Object(root)
}

#[test]
fn tdd_roundtrip_nested_document_and_reference() {
    let document = nested_document();
    let bytes = encode(&document).unwrap();
    assert!(bytes.starts_with(b"RDB\0\x01"));
    assert_eq!(decode(&bytes), Ok(document));
}

#[test]
fn tdd_all_primitive_types_roundtrip() {
    for value in [
        Value::Null, Value::Bool(false), Value::Bool(true),
        Value::Integer(i64::MIN), Value::Integer(i64::MAX),
        Value::Float(-25.125), Value::String(String::new()),
        Value::Bytes(vec![0, 255]), Value::Array(vec![]),
        Value::Object(BTreeMap::new()),
    ] {
        assert_eq!(decode(&encode(&value).unwrap()), Ok(value));
    }
}

#[test]
fn tdd_rejects_truncated_frames_and_trailing_data() {
    let frame = encode(&nested_document()).unwrap();
    for len in 0..frame.len() {
        assert!(decode(&frame[..len]).is_err(), "truncated at {len}");
    }
    let mut with_trailing = frame;
    with_trailing.push(0);
    assert_eq!(decode(&with_trailing), Err(CodecError::TrailingBytes));
}

#[test]
fn tdd_rejects_bad_magic_version_and_tag() {
    assert_eq!(decode(b"WRNG\x01\x00"), Err(CodecError::InvalidMagic));
    assert_eq!(decode(b"RDB\0\x02\x00"), Err(CodecError::UnsupportedVersion));
    assert_eq!(decode(b"RDB\0\x01\xff"), Err(CodecError::InvalidTag));
}

#[test]
fn tdd_rejects_non_finite_numbers_and_empty_refs() {
    assert_eq!(encode(&Value::Float(f64::NAN)), Err(CodecError::NonFiniteFloat));
    assert_eq!(encode(&Value::Reference { collection: "".into(), id: "x".into() }), Err(CodecError::InvalidReference));
}

#[test]
fn tdd_rejects_excessive_depth() {
    let mut item = Value::Null;
    for _ in 0..66 { item = Value::Array(vec![item]); }
    assert_eq!(encode(&item), Err(CodecError::TooDeep));
}

#[test]
fn tdd_rejects_duplicate_object_keys() {
    // 2 entries for same key: object tag + count + (len,key,null) twice.
    let mut bytes = b"RDB\0\x01\x08\x00\x00\x00\x02".to_vec();
    for _ in 0..2 { bytes.extend_from_slice(&[0, 0, 0, 1, b'x', 0]); }
    assert_eq!(decode(&bytes), Err(CodecError::DuplicateKey));
}
