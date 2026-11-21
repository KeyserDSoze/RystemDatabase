//! Strict JSON wire conversion. JSON is an interface format, never the on-disk format.
//! Tagged native types are parsed only with the explicit --typed-json opt-in.
use rystemdb_core::Value;
use serde_json::{Map, Number, Value as Json};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const WRAPPER_KEY: &str = "$rystem";

pub fn parse_document(input: &str, typed: bool) -> Result<Value, String> {
    let json: Json = serde_json::from_str(input).map_err(|err| format!("invalid JSON: {err}"))?;
    let value = from_json(json, typed)?;
    if !matches!(value, Value::Object(_)) {
        return Err("document root must be a JSON object".into());
    }
    Ok(value)
}

fn from_json(value: Json, typed: bool) -> Result<Value, String> {
    match value {
        Json::Null => Ok(Value::Null),
        Json::Bool(b) => Ok(Value::Bool(b)),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if n.is_f64() {
                let f = n.as_f64().ok_or("invalid floating-point value")?;
                if !f.is_finite() { return Err("non-finite number".into()); }
                Ok(Value::Float(f))
            } else {
                Err("integer is outside supported signed i64 range".into())
            }
        }
        Json::String(s) => Ok(Value::String(s)),
        Json::Array(items) => items.into_iter()
            .map(|item| from_json(item, typed))
            .collect::<Result<Vec<_>, _>>().map(Value::Array),
        Json::Object(mut entries) => {
            if typed && entries.len() == 1 {
                if let Some(wrapper) = entries.remove(WRAPPER_KEY) {
                    return parse_tag(wrapper);
                }
            }
            let mut object = BTreeMap::new();
            for (key, val) in entries {
                object.insert(key, from_json(val, typed)?);
            }
            Ok(Value::Object(object))
        }
    }
}

fn parse_tag(value: Json) -> Result<Value, String> {
    let Json::Object(mut fields) = value else {
        return Err("typed value must contain an object under $rystem".into());
    };
    if fields.remove("version") != Some(Json::Number(Number::from(1_u8))) {
        return Err("typed value requires version: 1".into());
    }
    let kind = fields.remove("type")
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or("typed value requires a string type")?;
    match kind.as_str() {
        "reference" => {
            if fields.len() != 2 { return Err("reference requires only collection and id".into()); }
            let collection = take_string(&mut fields, "collection")?;
            let id = take_string(&mut fields, "id")?;
            if collection.is_empty() || id.is_empty() {
                return Err("reference collection and id cannot be empty".into());
            }
            Ok(Value::Reference { collection, id })
        }
        "bytes" => {
            if fields.len() != 1 { return Err("bytes requires only a hex field".into()); }
            let hex = take_string(&mut fields, "hex")?;
            Ok(Value::Bytes(decode_hex(&hex)?))
        }
        "object" => {
            if fields.len() != 1 { return Err("object wrapper requires only a value field".into()); }
            let inner = fields.remove("value").ok_or("object wrapper requires value")?;
            let object = from_json(inner, false)?;
            if !matches!(object, Value::Object(_)) {
                return Err("object wrapper must contain a JSON object".into());
            }
            Ok(object)
        }
        _ => Err(format!("unsupported typed value: {kind}")),
    }
}

fn take_string(map: &mut Map<String, Json>, key: &str) -> Result<String, String> {
    map.remove(key)
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| format!("typed value requires string field {key}"))
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let bytes = hex.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err("hex byte representation must have even length".into());
    }
    let mut output = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let digit = |b: u8| -> Result<u8, String> {
            match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                b'A'..=b'F' => Ok(b - b'A' + 10),
                _ => Err("invalid hex byte representation".into()),
            }
        };
        output.push((digit(pair[0])? << 4) | digit(pair[1])?);
    }
    Ok(output)
}

fn tagged(mut payload: Map<String, Json>, tag: &str) -> Json {
    payload.insert("type".to_owned(), Json::String(tag.to_owned()));
    payload.insert("version".to_owned(), Json::Number(Number::from(1_u8)));
    let mut wrapper = Map::new();
    wrapper.insert(WRAPPER_KEY.to_owned(), Json::Object(payload));
    Json::Object(wrapper)
}

pub fn render_document(value: &Value) -> Result<String, String> {
    serde_json::to_string(&to_json(value)?).map_err(|e| format!("cannot serialize document: {e}"))
}

fn to_json(value: &Value) -> Result<Json, String> {
    match value {
        Value::Null => Ok(Json::Null),
        Value::Bool(x) => Ok(Json::Bool(*x)),
        Value::Integer(x) => Ok(Json::Number(Number::from(*x))),
        Value::Float(x) => Number::from_f64(*x)
            .map(Json::Number).ok_or_else(|| "non-finite floating-point value".to_string()),
        Value::String(x) => Ok(Json::String(x.clone())),
        Value::Bytes(bytes) => {
            let mut hex = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                write!(&mut hex, "{byte:02x}").map_err(|e| e.to_string())?;
            }
            let mut payload = Map::new();
            payload.insert("hex".to_string(), Json::String(hex));
            Ok(tagged(payload, "bytes"))
        }
        Value::Reference { collection, id } => {
            let mut payload = Map::new();
            payload.insert("collection".to_string(), Json::String(collection.clone()));
            payload.insert("id".to_string(), Json::String(id.clone()));
            Ok(tagged(payload, "reference"))
        }
        Value::Array(values) => values.iter()
            .map(to_json).collect::<Result<Vec<_>, _>>().map(Json::Array),
        Value::Object(fields) => {
            let mut obj = Map::new();
            for (key, val) in fields {
                obj.insert(key.clone(), to_json(val)?);
            }
            Ok(Json::Object(obj))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_input_is_explicit_and_escaped_object_preserves_literal_special_key() {
        let plain = r#"{"x":{"$rystem":{"version":1,"type":"reference","collection":"u","id":"1"}}}"#;
        let Value::Object(root) = parse_document(plain, false).unwrap() else { panic!("map") };
        assert!(matches!(root.get("x"), Some(Value::Object(_))));
        let Value::Object(root) = parse_document(plain, true).unwrap() else { panic!("map") };
        assert!(matches!(root.get("x"), Some(Value::Reference { .. })));
        let escaped = r#"{"x":{"$rystem":{"version":1,"type":"object","value":{"$rystem":{"version":1,"type":"reference","collection":"u","id":"1"}}}}}"#;
        let Value::Object(root) = parse_document(escaped, true).unwrap() else { panic!("map") };
        assert!(matches!(root.get("x"), Some(Value::Object(_))));
    }

    #[test]
    fn rejects_large_unsigned_integers_not_silently_rounded_to_float() {
        assert!(parse_document(r#"{"x":9223372036854775808}"#, false).is_err());
        assert!(parse_document(r#"{"x":1e1000}"#, false).is_err());
    }

    #[test]
    fn strict_hex_and_typed_reference() {
        for hex in ["z0", "f", "00xx", "ffffx"] {
            let json = format!(r#"{{"x":{{"$rystem":{{"version":1,"type":"bytes","hex":"{hex}"}}}}}}"#);
            assert!(parse_document(&json, true).is_err());
        }
        let val = Value::Bytes(vec![0, 255, 128]);
        let output = render_document(&val).unwrap();
        assert_eq!(output, r#"{"$rystem":{"hex":"00ff80","type":"bytes","version":1}}"#);
    }
}
