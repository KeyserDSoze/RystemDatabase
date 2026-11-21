//! Experimental loopback-only RSWP RPC server.
//!
//! Security: intentionally NO TLS/auth yet. Refuse every non-loopback binding.
//! The server owns ONE DocumentStore for its whole lifetime, and all clients
//! share it behind a mutex. A bounded fixed worker pool prevents unlimited
//! threads from untrusted socket connections. Not a public network database.
use rystemdb_core::{decode as decode_document, encode as encode_document, Value};
use rystemdb_protocol::{read_frame, write_frame, Frame, Kind, Operation, MAX_PAYLOAD_BYTES};
use rystemdb_storage::DocumentStore;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

const WORKERS: usize = 8;
const BACKLOG: usize = 32;
const MAX_REQUESTS_PER_CONNECTION: usize = 128;
const TIMEOUT: Duration = Duration::from_secs(3);

const OK: u16 = 0;
const NOT_FOUND: u16 = 1;
const BAD_REQUEST: u16 = 2;
const INTERNAL: u16 = 4;

/// Starts the loopback-only binary service. The default health-only HTTP server
/// remains a separate CLI command and keeps its original behavior.
pub fn run(args: &[String]) -> Result<(), String> {
    let (mut data, mut listen) = (None, None);
    let mut pos = 0;
    while pos < args.len() {
        let flag = args[pos].as_str();
        let value = args.get(pos + 1).ok_or(format!("missing value for {flag}"))?;
        let target = match flag {
            "--data" => &mut data,
            "--listen" => &mut listen,
            _ => return Err(format!("unknown rpc-serve argument: {flag}")),
        };
        if target.replace(value.as_str()).is_some() {
            return Err(format!("duplicate rpc-serve argument: {flag}"));
        }
        pos += 2;
    }
    let data = data.ok_or("rpc-serve requires --data PATH")?;
    if data.is_empty() { return Err("--data cannot be empty".into()); }
    let addr: SocketAddr = listen.unwrap_or("127.0.0.1:8790").parse()
        .map_err(|_| "--listen must be a numeric IP:port (e.g. 127.0.0.1:8790)")?;
    // Critical: validation precedes DocumentStore::open, so an insecure
    // attempted bind cannot even create a new database directory.
    if !addr.ip().is_loopback() {
        return Err("rpc-serve is restricted to loopback; TLS/auth not implemented".into());
    }

    let listener = TcpListener::bind(addr).map_err(|err| format!("RPC bind failed: {err}"))?;
    let db = Arc::new(Mutex::new(
        DocumentStore::open(data).map_err(|err| format!("RPC database open failed: {err}"))?
    ));
    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(BACKLOG);
    let shared_rx = Arc::new(Mutex::new(receiver));
    for _ in 0..WORKERS {
        let db = Arc::clone(&db);
        let rx = Arc::clone(&shared_rx);
        thread::spawn(move || loop {
            let accepted = {
                let Ok(receiver) = rx.lock() else { break };
                receiver.recv()
            };
            let Ok(stream) = accepted else { break };
            handle_client(stream, &db);
        });
    }
    println!("Rystem Database experimental LOOPBACK RPC listening on {}",
        listener.local_addr().map_err(|err| err.to_string())?);

    for accepted in listener.incoming() {
        match accepted {
            Ok(stream) => {
                // Drop clients promptly if the bounded worker queue is full.
                let _ = sender.try_send(stream);
            }
            Err(err) => eprintln!("RPC accept error: {err}"),
        }
    }
    Ok(())
}

fn handle_client(mut stream: TcpStream, db: &Mutex<DocumentStore>) {
    if stream.set_read_timeout(Some(TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(TIMEOUT)).is_err()
    {
        return;
    }
    let _ = stream.set_nodelay(true);
    let mut greeted = false;
    for _ in 0..MAX_REQUESTS_PER_CONNECTION {
        let request = match read_frame(&mut stream) {
            Ok(request) => request,
            Err(_) => break, // malformed/version-mismatched/partial/closed: fail closed
        };
        if request.kind != Kind::Request {
            break;
        }
        let (status, payload, close) = if !greeted {
            if request.operation != Operation::Hello || request.payload != [1, 0] {
                (BAD_REQUEST, Vec::new(), true)
            } else {
                greeted = true;
                (OK, vec![1,0], false)
            }
        } else {
            match request.operation {
                Operation::Hello => (BAD_REQUEST, Vec::new(), true),
                Operation::Ping if request.payload.is_empty() => (OK, Vec::new(), false),
                Operation::Ping => (BAD_REQUEST, Vec::new(), false),
                Operation::Put | Operation::Get | Operation::Delete =>
                    handle_document(&request, db),
            }
        };
        // Error bodies intentionally remain empty: don't disclose file paths,
        // nested data or internal error strings over an unauthenticated link.
        let response = Frame::response(request.operation, request.request_id, status, payload);
        if write_frame(&mut stream, &response).is_err() { break; }
        if close { break; }
    }
}

fn handle_document(request: &Frame, db: &Mutex<DocumentStore>) -> (u16, Vec<u8>, bool) {
    let Some((key, payload_offset)) = parse_key(&request.payload) else {
        return (BAD_REQUEST, Vec::new(), false);
    };
    // Lock the *single* writer for both reads and mutations. Per-connection
    // DocumentStore::open would violate exclusive on-disk writer ownership.
    match request.operation {
        Operation::Put => {
            let doc = match decode_document(&request.payload[payload_offset..]) {
                Ok(Value::Object(map)) => Value::Object(map),
                _ => return (BAD_REQUEST, Vec::new(), false),
            };
            match db.lock() {
                Ok(mut store) => match store.put(&key, &doc) {
                    Ok(()) => (OK, Vec::new(), false),
                    Err(_) => (INTERNAL, Vec::new(), false),
                },
                Err(_) => (INTERNAL, Vec::new(), false),
            }
        }
        Operation::Get => {
            if payload_offset != request.payload.len() {
                return (BAD_REQUEST, Vec::new(), false);
            }
            match db.lock() {
                Ok(store) => match store.get(&key) {
                    Ok(Some(value)) => match encode_document(&value) {
                        Ok(bytes) if bytes.len() <= MAX_PAYLOAD_BYTES => (OK, bytes, false),
                        _ => (INTERNAL, Vec::new(), false),
                    },
                    Ok(None) => (NOT_FOUND, Vec::new(), false),
                    Err(_) => (INTERNAL, Vec::new(), false),
                },
                Err(_) => (INTERNAL, Vec::new(), false),
            }
        }
        Operation::Delete => {
            if payload_offset != request.payload.len() {
                return (BAD_REQUEST, Vec::new(), false);
            }
            match db.lock() {
                Ok(mut store) => match store.delete(&key) {
                    Ok(true) => (OK, Vec::new(), false),
                    Ok(false) => (NOT_FOUND, Vec::new(), false),
                    Err(_) => (INTERNAL, Vec::new(), false),
                },
                Err(_) => (INTERNAL, Vec::new(), false),
            }
        }
        _ => (BAD_REQUEST, Vec::new(), false),
    }
}

/// v1 CRUD body layout:
/// u16 BE collection_length, ASCII collection (1..128 [A-Za-z0-9_-]),
/// u16 BE id_length, UTF-8 ID (nonempty, no control chars),
/// followed by RDB\0 typed document bytes only for PUT.
/// Returns the composite storage key and offset of the typed document body.
fn parse_key(bytes: &[u8]) -> Option<(String, usize)> {
    let collection_len = u16::from_be_bytes(bytes.get(..2)?.try_into().ok()?) as usize;
    if !(1..=128).contains(&collection_len) { return None; }
    let collection_bytes = bytes.get(2..2+collection_len)?;
    if !collection_bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-') {
        return None;
    }
    let offset = 2 + collection_len;
    let id_len = u16::from_be_bytes(bytes.get(offset..offset+2)?.try_into().ok()?) as usize;
    if id_len == 0 || collection_len + 1 + id_len > 1024 { return None; }
    let begin = offset + 2;
    let id = std::str::from_utf8(bytes.get(begin..begin+id_len)?).ok()?;
    if id.chars().any(char::is_control) { return None; }
    let collection = std::str::from_utf8(collection_bytes).ok()?;
    Some((format!("{collection}/{id}"), begin+id_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_key_layout_is_exact_and_does_not_parse_json() {
        let mut body = vec![0,5];
        body.extend_from_slice(b"users");
        body.extend_from_slice(&[0,3]);
        body.extend_from_slice(b"u42");
        assert_eq!(parse_key(&body), Some(("users/u42".into(), 12)));
        assert_eq!(parse_key(&body[..5]), None);
        body[1] = 0xff;
        assert_eq!(parse_key(&body), None);
    }

    #[test]
    fn never_accept_control_chars_or_invalid_collection_names() {
        assert_eq!(parse_key(&[0,1,b'/',0,1,b'x']), None);
        assert_eq!(parse_key(&[0,1,b'u',0,1,0]), None);
    }
}
