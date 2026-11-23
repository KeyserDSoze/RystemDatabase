//! Rystem binary document codec foundation (format version 1).
//! The v1 codec is self-describing and intentionally not yet optimized with a schema dictionary.
//! It is NOT a transactional persistence or query engine.
use std::collections::BTreeMap;
use std::fmt;

const MAGIC: &[u8; 4] = b"RDB\0";
const VERSION: u8 = 1;
const MAX_DEPTH: usize = 64;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
// #71: cap decoded heap-value expansion, not only on-wire byte count.
const MAX_VALUE_NODES: usize = 262_144;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
    Reference { collection: String, id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    UnexpectedEof,
    InvalidMagic,
    UnsupportedVersion,
    InvalidTag,
    InvalidUtf8,
    TooDeep,
    TooLarge,
    TrailingBytes,
    DuplicateKey,
    NonCanonicalKeyOrder,
    NonFiniteFloat,
    InvalidReference,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CodecError {}

/// Serialize a typed document to a portable, big-endian binary frame.
pub fn encode(value: &Value) -> Result<Vec<u8>, CodecError> {
    let mut output = Vec::new();
    append(&mut output, MAGIC)?;
    append(&mut output, &[VERSION])?;
    let mut nodes = 0;
    encode_value(value, &mut output, 0, &mut nodes)?;
    Ok(output)
}

/// Deserialize exactly one complete binary frame. Truncated and trailing bytes are rejected.
pub fn decode(input: &[u8]) -> Result<Value, CodecError> {
    if input.len() > MAX_DOCUMENT_BYTES {
        return Err(CodecError::TooLarge);
    }
    if input.len() < 5 {
        return Err(CodecError::UnexpectedEof);
    }
    if &input[..4] != MAGIC {
        return Err(CodecError::InvalidMagic);
    }
    if input[4] != VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    let mut reader = Reader { input, offset: 5, nodes: 0 };
    let value = reader.value(0)?;
    if reader.offset != input.len() {
        return Err(CodecError::TrailingBytes);
    }
    Ok(value)
}

fn append(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CodecError> {
    let total = out.len().checked_add(bytes.len()).ok_or(CodecError::TooLarge)?;
    if total > MAX_DOCUMENT_BYTES {
        return Err(CodecError::TooLarge);
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_len(out: &mut Vec<u8>, size: usize) -> Result<(), CodecError> {
    let len = u32::try_from(size).map_err(|_| CodecError::TooLarge)?;
    append(out, &len.to_be_bytes())
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), CodecError> {
    write_len(out, value.len())?;
    append(out, value.as_bytes())
}

// Every tagged value (including a root array/object) consumes one slot.
// For collections, check the *minimum* number of new nodes before iteration
// or any preallocation. Nested children share the same counter.
fn take_node(nodes: &mut usize) -> Result<(), CodecError> {
    if *nodes >= MAX_VALUE_NODES {
        return Err(CodecError::TooLarge);
    }
    *nodes += 1;
    Ok(())
}

fn check_children(nodes: usize, count: usize) -> Result<(), CodecError> {
    if count > MAX_VALUE_NODES - nodes {
        return Err(CodecError::TooLarge);
    }
    Ok(())
}

fn encode_value(value: &Value, out: &mut Vec<u8>, depth: usize, nodes: &mut usize) -> Result<(), CodecError> {
    if depth > MAX_DEPTH {
        return Err(CodecError::TooDeep);
    }
    take_node(nodes)?;
    match value {
        Value::Null => append(out, &[0]),
        Value::Bool(false) => append(out, &[1]),
        Value::Bool(true) => append(out, &[2]),
        Value::Integer(x) => { append(out, &[3])?; append(out, &x.to_be_bytes()) }
        Value::Float(x) => {
            if !x.is_finite() { return Err(CodecError::NonFiniteFloat); }
            append(out, &[4])?;
            append(out, &x.to_bits().to_be_bytes())
        }
        Value::String(x) => { append(out, &[5])?; write_string(out, x) }
        Value::Bytes(x) => { append(out, &[6])?; write_len(out, x.len())?; append(out, x) }
        Value::Array(values) => {
            check_children(*nodes, values.len())?;
            append(out, &[7])?;
            write_len(out, values.len())?;
            for element in values { encode_value(element, out, depth + 1, nodes)?; }
            Ok(())
        }
        Value::Object(fields) => {
            check_children(*nodes, fields.len())?;
            append(out, &[8])?;
            write_len(out, fields.len())?;
            for (name, value) in fields {
                write_string(out, name)?;
                encode_value(value, out, depth + 1, nodes)?;
            }
            Ok(())
        }
        Value::Reference { collection, id } => {
            if collection.is_empty() || id.is_empty() { return Err(CodecError::InvalidReference); }
            append(out, &[9])?;
            write_string(out, collection)?;
            write_string(out, id)
        }
    }
}

struct Reader<'a> {
    input: &'a [u8],
    offset: usize,
    nodes: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], CodecError> {
        let end = self.offset.checked_add(size).ok_or(CodecError::TooLarge)?;
        let chunk = self.input.get(self.offset..end).ok_or(CodecError::UnexpectedEof)?;
        self.offset = end;
        Ok(chunk)
    }

    fn length(&mut self) -> Result<usize, CodecError> {
        let raw: [u8; 4] = self.take(4)?.try_into().map_err(|_| CodecError::UnexpectedEof)?;
        Ok(u32::from_be_bytes(raw) as usize)
    }

    fn string(&mut self) -> Result<String, CodecError> {
        let size = self.length()?;
        String::from_utf8(self.take(size)?.to_vec()).map_err(|_| CodecError::InvalidUtf8)
    }

    fn value(&mut self, depth: usize) -> Result<Value, CodecError> {
        if depth > MAX_DEPTH { return Err(CodecError::TooDeep); }
        take_node(&mut self.nodes)?;
        let tag = self.take(1)?[0];
        match tag {
            0 => Ok(Value::Null),
            1 => Ok(Value::Bool(false)),
            2 => Ok(Value::Bool(true)),
            3 => {
                let raw: [u8; 8] = self.take(8)?.try_into().map_err(|_| CodecError::UnexpectedEof)?;
                Ok(Value::Integer(i64::from_be_bytes(raw)))
            }
            4 => {
                let raw: [u8; 8] = self.take(8)?.try_into().map_err(|_| CodecError::UnexpectedEof)?;
                let value = f64::from_bits(u64::from_be_bytes(raw));
                if !value.is_finite() { return Err(CodecError::NonFiniteFloat); }
                Ok(Value::Float(value))
            }
            5 => Ok(Value::String(self.string()?)),
            6 => {
                let size = self.length()?;
                Ok(Value::Bytes(self.take(size)?.to_vec()))
            }
            7 => {
                let count = self.length()?;
                check_children(self.nodes, count)?;
                if count > self.input.len().saturating_sub(self.offset) {
                    return Err(CodecError::UnexpectedEof);
                }
                let mut result = Vec::new();
                for _ in 0..count { result.push(self.value(depth + 1)?); }
                Ok(Value::Array(result))
            }
            8 => {
                let count = self.length()?;
                check_children(self.nodes, count)?;
                if count > self.input.len().saturating_sub(self.offset) / 5 {
                    return Err(CodecError::UnexpectedEof);
                }
                let mut result: BTreeMap<String, Value> = BTreeMap::new();
                for _ in 0..count {
                    let key = self.string()?;
                    // #78: Validate the key BEFORE decoding the (potentially
                    // huge) child value. Existing map keys are already in
                    // strict UTF-8 byte order (#65). Borrow the greatest key
                    // instead of cloning every field name just to remember it.
                    if let Some((previous, _)) = result.last_key_value() {
                        if key.as_str() <= previous.as_str() {
                            // A duplicate can be nonadjacent: a, c, a must
                            // remain DuplicateKey, not NonCanonicalKeyOrder.
                            if key.as_str() == previous.as_str()
                                || result.contains_key(key.as_str())
                            {
                                return Err(CodecError::DuplicateKey);
                            }
                            return Err(CodecError::NonCanonicalKeyOrder);
                        }
                    }
                    let val = self.value(depth + 1)?;
                    result.insert(key, val);
                }
                Ok(Value::Object(result))
            }
            9 => {
                let collection = self.string()?;
                let id = self.string()?;
                if collection.is_empty() || id.is_empty() { return Err(CodecError::InvalidReference); }
                Ok(Value::Reference { collection, id })
            }
            _ => Err(CodecError::InvalidTag),
        }
    }
}
