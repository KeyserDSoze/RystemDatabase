use rystemdb_core::{decode, encode, Value};
use std::collections::BTreeMap;

const GOLDEN: &[u8] = b"RDB\0\x01\x08\x00\x00\x00\x03\
\x00\x00\x00\x01a\x07\x00\x00\x00\x03\x00\x02\x03\xff\xff\xff\xff\xff\xff\xff\xff\
\x00\x00\x00\x01b\x06\x00\x00\x00\x03\x00\xff\x80\
\x00\x00\x00\x01r\x09\x00\x00\x00\x05users\x00\x00\x00\x03u42";

#[test]
fn tdd_typed_nested_golden_cross_language() {
    let doc=Value::Object(BTreeMap::from([
        ("a".into(),Value::Array(vec![Value::Null,Value::Bool(true),Value::Integer(-1)])),
        ("b".into(),Value::Bytes(vec![0,255,128])),
        ("r".into(),Value::Reference{collection:"users".into(),id:"u42".into()}),
    ]));
    assert_eq!(encode(&doc).unwrap(),GOLDEN);
    assert_eq!(decode(GOLDEN).unwrap(),doc);
}
#[test]
fn tdd_typed_document_order_and_malformed_binary() {
    let doc=Value::Object(BTreeMap::from([
        ("\u{e000}".into(),Value::String("bmp".into())),
        ("😀".into(),Value::String("astral".into())),
        ("n".into(),Value::Integer(i64::MIN)),
        ("maybe".into(),Value::Null),
    ]));
    assert_eq!(decode(&encode(&doc).unwrap()).unwrap(),doc);
    let mut dup=hex_vec("52444200010800000002000000017800000000017802");
    assert!(decode(&dup).is_err());
    dup[0]=0;
    assert!(decode(&dup).is_err());
}
fn hex_vec(s:&str)->Vec<u8>{
    (0..s.len()).step_by(2).map(|i|u8::from_str_radix(&s[i..i+2],16).unwrap()).collect()
}

#[test]
fn tdd_unicode_bom_is_not_stripped_from_rdb_string_fields() {
    let doc=Value::Object(BTreeMap::from([
        ("\u{feff}name".into(),Value::String("\u{feff}value".into())),
        ("reference".into(),Value::Reference{
            collection:"users".into(),id:"\u{feff}id".into()
        })
    ]));
    assert_eq!(decode(&encode(&doc).unwrap()).unwrap(),doc);
}
