//! Experimental loopback-only RSWP RPC server.
//!
//! Security: optional TLS1.3 + local ACL. No secure remote bindings yet; deny all nonloopback.
//! The server owns ONE DocumentStore for its whole lifetime, and all clients
//! share it behind a mutex. A bounded fixed worker pool prevents unlimited
//! threads from untrusted socket connections. Not a public network database.
use crate::auth::{AuthPolicy, Grant};
use crate::tls;
use rystemdb_core::{decode as decode_document, encode as encode_document, Value};
use rystemdb_protocol::{read_frame, write_frame, Frame, Kind, Operation, MAX_PAYLOAD_BYTES};
use rystemdb_storage::DocumentStore;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
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
const UNAUTHORIZED: u16 = 6;

/// Starts the loopback-only binary service. The default health-only HTTP server
/// remains a separate CLI command and keeps its original behavior.
pub fn run(args: &[String]) -> Result<(), String> {
    let (mut data, mut listen, mut auth_file, mut tls_cert, mut tls_key) = (None, None, None, None, None);
    let mut pos = 0;
    while pos < args.len() {
        let flag = args[pos].as_str();
        let value = args.get(pos + 1).ok_or(format!("missing value for {flag}"))?;
        let target = match flag {
            "--data" => &mut data,
            "--listen" => &mut listen,
            "--auth-file" => &mut auth_file,
            "--tls-cert" => &mut tls_cert,
            "--tls-key" => &mut tls_key,
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
        return Err("rpc-serve is restricted to loopback; secure remote transport not yet enabled".into());
    }

    // Fail closed on invalid credentials/permissions BEFORE binding the socket
    // or opening/creating any database files.
    let policy = match auth_file {
        Some(path) => Some(Arc::new(AuthPolicy::load(Path::new(path))?)),
        None => None, // backward-compatible, localhost-only developer mode
    };
    // A TLS endpoint must also have explicitly defined per-collection ACLs;
    // no anonymous TLS (and never a plaintext fallback) in this mode.
    if tls_cert.is_some() != tls_key.is_some() {
        return Err("TLS requires both --tls-cert and --tls-key".into());
    }
    if tls_cert.is_some() && policy.is_none() {
        return Err("TLS mode requires --auth-file with explicit collection permissions".into());
    }
    let tls_config = match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => Some(tls::server_config(Path::new(cert), Path::new(key))?),
        _ => None,
    };
    let listener = TcpListener::bind(addr).map_err(|err| format!("RPC bind failed: {err}"))?;
    let db = Arc::new(Mutex::new(
        DocumentStore::open(data).map_err(|err| format!("RPC database open failed: {err}"))?
    ));
    let (sender, receiver) = mpsc::sync_channel::<TcpStream>(BACKLOG);
    let shared_rx = Arc::new(Mutex::new(receiver));
    for _ in 0..WORKERS {
        let db = Arc::clone(&db);
        let rx = Arc::clone(&shared_rx);
        let policy = policy.clone();
        let tls_config = tls_config.clone();
        thread::spawn(move || loop {
            let accepted = {
                let Ok(receiver) = rx.lock() else { break };
                receiver.recv()
            };
            let Ok(stream) = accepted else { break };
            handle_client(stream, &db, policy.as_deref(), tls_config.as_ref());
        });
    }
    println!("Rystem Database experimental LOOPBACK RPC listening on {}",
        listener.local_addr().map_err(|err| err.to_string())?);
    // Explicit flush makes stdout a deterministic readiness channel even
    // when it is piped to a test process instead of a terminal.
    std::io::stdout().flush().map_err(|err| err.to_string())?;

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

fn handle_client(mut stream: TcpStream, db: &Mutex<DocumentStore>,
    policy: Option<&AuthPolicy>, tls: Option<&Arc<rustls::ServerConfig>>)
{
    if stream.set_read_timeout(Some(TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(TIMEOUT)).is_err()
    {
        return;
    }
    let _ = stream.set_nodelay(true);
    if let Some(config) = tls {
        let Ok(conn) = rustls::ServerConnection::new(Arc::clone(config)) else {
            return;
        };
        let mut wrapped = rustls::StreamOwned::new(conn, stream);
        handle_client_frames(&mut wrapped, db, policy);
    } else {
        handle_client_frames(&mut stream, db, policy);
    }
}

fn handle_client_frames<S: Read + Write>(
    stream: &mut S, db: &Mutex<DocumentStore>, policy: Option<&AuthPolicy>
) {
    let mut greeted = false;
    let mut grants: Option<Vec<Grant>> = None;
    for _ in 0..MAX_REQUESTS_PER_CONNECTION {
        let request = match read_frame(&mut stream) {
            Ok(request) => request,
            Err(_) => break, // malformed/version-mismatched/partial/closed: fail closed
        };
        if request.kind != Kind::Request {
            break;
        }
        let (status, payload, close) = if !greeted {
            if request.operation != Operation::Hello {
                (BAD_REQUEST, Vec::new(), true)
            } else if let Some(policy) = policy {
                if let Some(allowed) = policy.authenticate(&request.payload) {
                    grants = Some(allowed);
                    greeted = true;
                    (OK, vec![1, 0], false)
                } else {
                    (UNAUTHORIZED, Vec::new(), true)
                }
            } else if request.payload != [1, 0] {
                (BAD_REQUEST, Vec::new(), true)
            } else {
                greeted = true;
                (OK, vec![1, 0], false)
            }
        } else {
            match request.operation {
                Operation::Hello => (BAD_REQUEST, Vec::new(), true),
                Operation::Ping if request.payload.is_empty() => (OK, Vec::new(), false),
                Operation::Ping => (BAD_REQUEST, Vec::new(), false),
                Operation::Put | Operation::Get | Operation::Delete =>
                    handle_document(&request, db, grants.as_deref()),
            }
        };
        // Error bodies intentionally remain empty: don't disclose credentials,
        // paths, nested data or internal errors even on authorization failures.
        let response = Frame::response(request.operation, request.request_id, status, payload);
        if write_frame(&mut stream, &response).is_err() { break; }
        if close { break; }
    }
}

fn handle_document(request: &Frame, db: &Mutex<DocumentStore>, grants: Option<&[Grant]>) -> (u16, Vec<u8>, bool) {
    let Some((key, payload_offset)) = parse_key(&request.payload) else {
        return (BAD_REQUEST, Vec::new(), false);
    };
    if let Some(grants) = grants {
        let collection = key.split_once('/').map(|(name, _)| name).unwrap_or("");
        if !grants.iter().any(|grant| grant.allows(collection, request.operation)) {
            // Deny before reading or mutating the shared store; never reveal
            // whether an unauthorized collection/document exists.
            return (UNAUTHORIZED, Vec::new(), false);
        }
    }
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

/// Parse using the shared RSWP binary key codec, retaining the offset
/// of an optional typed PUT document body. No duplicate key grammar.
fn parse_key(bytes: &[u8]) -> Option<(String, usize)> {
    let parsed = rystemdb_protocol::crud::decode_key(bytes).ok()?;
    let offset = bytes.len() - parsed.document.len();
    Some((format!("{}/{}", parsed.collection, parsed.id), offset))
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
