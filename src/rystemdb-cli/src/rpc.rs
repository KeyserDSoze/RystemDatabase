//! Experimental loopback-only RSWP RPC server.
//!
//! Security: optional TLS1.3 + local ACL. No secure remote bindings yet; deny all nonloopback.
//! The server owns ONE DocumentStore for its whole lifetime, and all clients
//! share it behind a mutex. A bounded fixed worker pool prevents unlimited
//! threads from untrusted socket connections. Not a public network database.
use crate::auth::{grants_for_mtls_principal, load_mtls_principal_policy, AuthPolicy, Grant};
use crate::tls;
use rystemdb_core::{decode as decode_document, encode as encode_document, Value};
use rystemdb_protocol::{read_frame, read_frame_with_limit, write_frame, Frame, Kind, Operation, MAX_PAYLOAD_BYTES};
use rystemdb_storage::DocumentStore;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const WORKERS: usize = 8;
const BACKLOG: usize = 32;
const MAX_REQUESTS_PER_CONNECTION: usize = 128;
const TIMEOUT: Duration = Duration::from_secs(3);
// #72: HELLO is either two version bytes (local developer mode) or
// 1,0 + u16(32) + a 32-byte bearer token. Reject a larger *declared*
// pre-authentication frame before allocating its body or holding a worker.
const MAX_PRE_AUTH_PAYLOAD_BYTES: usize = 36;


// #73: one accept-to-HELLO absolute deadline through TLS/mTLS handshakes.
// #77: every later complete RSWP request has its own absolute *ingress*
// deadline. Both are enforced at the underlying socket layer so repeated
// read_exact/rustls reads cannot restart the clock. Disk work and response
// writes retain their previous (separate) per-I/O timeouts.
struct DeadlineSocket {
    socket: TcpStream,
    pre_auth_until: Option<Instant>,
    frame_until: Option<Instant>,
}

impl DeadlineSocket {
    fn new(socket: TcpStream, accepted_at: Instant) -> io::Result<Self> {
        socket.set_read_timeout(Some(TIMEOUT))?;
        socket.set_write_timeout(Some(TIMEOUT))?;
        let _ = socket.set_nodelay(true);
        Ok(Self {
            socket,
            pre_auth_until: Some(accepted_at + TIMEOUT),
            frame_until: None,
        })
    }

    fn remaining(&self) -> io::Result<Option<Duration>> {
        // These deadlines never overlap: HELLO releases its budget before
        // accepting any subsequent RSWP frame.
        self.pre_auth_until.or(self.frame_until).map(|until| {
            until.checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| io::Error::new(
                    io::ErrorKind::TimedOut, "RSWP handshake/frame ingress deadline expired"
                ))
        }).transpose()
    }

    fn before_read(&self) -> io::Result<()> {
        if let Some(remaining) = self.remaining()? {
            self.socket.set_read_timeout(Some(remaining))?;
        }
        Ok(())
    }

    fn before_write(&self) -> io::Result<()> {
        if let Some(remaining) = self.remaining()? {
            self.socket.set_write_timeout(Some(remaining))?;
        }
        Ok(())
    }

    fn finish_pre_auth(&mut self) -> io::Result<()> {
        // Do not let a peer authenticate AFTER the absolute deadline and
        // restore an unlimited session by completing HELLO too late.
        self.remaining()?;
        self.socket.set_read_timeout(Some(TIMEOUT))?;
        self.socket.set_write_timeout(Some(TIMEOUT))?;
        self.pre_auth_until = None;
        Ok(())
    }

    fn begin_frame_receive(&mut self) -> io::Result<()> {
        if self.pre_auth_until.is_some() || self.frame_until.is_some() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "cannot overlap RPC frame ingress deadlines"));
        }
        self.frame_until = Some(Instant::now() + TIMEOUT);
        Ok(())
    }

    fn finish_frame_receive(&mut self) -> io::Result<()> {
        if self.frame_until.is_none() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "no active RPC frame ingress deadline"));
        }
        // A frame that finished only after its deadline is not accepted.
        self.remaining()?;
        self.frame_until = None;
        // Do not apply the ingress budget to backend storage/fsync or to
        // sending the reply. Preserve the existing per-I/O socket limits.
        self.socket.set_read_timeout(Some(TIMEOUT))
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.socket.set_read_timeout(timeout)
    }
}

impl Read for DeadlineSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.before_read()?;
        self.socket.read(buf)
    }
}

impl Write for DeadlineSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.before_write()?;
        self.socket.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.before_write()?;
        self.socket.flush()
    }
}

// Both plaintext and rustls must apply deadlines to the underlying socket.
// HELLO's accept-time budget ends only after its reply was flushed; each
// later frame receives an independent read-only budget (#77).
trait PreAuthTransport: Read + Write {
    fn finish_pre_auth(&mut self) -> io::Result<()>;
    fn begin_frame_receive(&mut self) -> io::Result<()>;
    fn finish_frame_receive(&mut self) -> io::Result<()>;
}

impl PreAuthTransport for DeadlineSocket {
    fn finish_pre_auth(&mut self) -> io::Result<()> {
        DeadlineSocket::finish_pre_auth(self)
    }
    fn begin_frame_receive(&mut self) -> io::Result<()> {
        DeadlineSocket::begin_frame_receive(self)
    }
    fn finish_frame_receive(&mut self) -> io::Result<()> {
        DeadlineSocket::finish_frame_receive(self)
    }
}

impl PreAuthTransport for rustls::StreamOwned<rustls::ServerConnection, DeadlineSocket> {
    fn finish_pre_auth(&mut self) -> io::Result<()> {
        self.sock.finish_pre_auth()
    }
    fn begin_frame_receive(&mut self) -> io::Result<()> {
        self.sock.begin_frame_receive()
    }
    fn finish_frame_receive(&mut self) -> io::Result<()> {
        self.sock.finish_frame_receive()
    }
}

const OK: u16 = 0;
const NOT_FOUND: u16 = 1;
const BAD_REQUEST: u16 = 2;
const INTERNAL: u16 = 4;
const UNAUTHORIZED: u16 = 6;

/// Starts the loopback-only binary service. The default health-only HTTP server
/// remains a separate CLI command and keeps its original behavior.
pub fn run(args: &[String]) -> Result<(), String> {
    let (mut data, mut listen, mut auth_file, mut tls_cert, mut tls_key, mut mtls_policy, mut mtls_ca) =
        (None, None, None, None, None, None, None);
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
            "--mtls-policy" => &mut mtls_policy,
            "--mtls-ca" => &mut mtls_ca,
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

    // An mTLS principal policy must NEVER silently enable a bearer-only TLS
    // server. Validate before opening storage or binding to any address.
    // A CA without the principal ACL is also forbidden.
    let mut mtls_rules: Option<Arc<Vec<(String, Grant)>>> = None;
    let mtls_ca = match (mtls_policy, mtls_ca) {
        (Some(policy_path), Some(ca_path)) => {
            let rules = load_mtls_principal_policy(Path::new(policy_path))?;
            if auth_file.is_none() || tls_cert.is_none() || tls_key.is_none() {
                return Err("mTLS requires --auth-file, --tls-cert and --tls-key".into());
            }
            mtls_rules = Some(Arc::new(rules));
            Some(Path::new(ca_path))
        }
        (Some(policy_path), None) => {
            // Preserve #57's fail-closed validation-only contract.
            load_mtls_principal_policy(Path::new(policy_path))?;
            return Err("mTLS client certificate authentication is not yet enabled".into());
        }
        (None, Some(_)) => return Err("mTLS requires --mtls-policy".into()),
        (None, None) => None,
    };

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
        (Some(cert), Some(key)) => Some(tls::server_config(Path::new(cert), Path::new(key), mtls_ca)?),
        _ => None,
    };
    let listener = TcpListener::bind(addr).map_err(|err| format!("RPC bind failed: {err}"))?;
    let db = Arc::new(Mutex::new(
        DocumentStore::open(data).map_err(|err| format!("RPC database open failed: {err}"))?
    ));
    let (sender, receiver) = mpsc::sync_channel::<(TcpStream, Instant)>(BACKLOG);
    let shared_rx = Arc::new(Mutex::new(receiver));
    for _ in 0..WORKERS {
        let db = Arc::clone(&db);
        let rx = Arc::clone(&shared_rx);
        let policy = policy.clone();
        let tls_config = tls_config.clone();
        let mtls_rules = mtls_rules.clone();
        thread::spawn(move || loop {
            let accepted = {
                let Ok(receiver) = rx.lock() else { break };
                receiver.recv()
            };
            let Ok((stream, accepted_at)) = accepted else { break };
            handle_client(stream, accepted_at, &db, policy.as_deref(), tls_config.as_ref(),
                mtls_rules.as_deref().map(Vec::as_slice));
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
                let _ = sender.try_send((stream, Instant::now()));
            }
            Err(err) => eprintln!("RPC accept error: {err}"),
        }
    }
    Ok(())
}

fn handle_client(stream: TcpStream, accepted_at: Instant, db: &Mutex<DocumentStore>,
    policy: Option<&AuthPolicy>, tls: Option<&Arc<rustls::ServerConfig>>,
    mtls_rules: Option<&[(String, Grant)]>)
{
    let Ok(mut stream) = DeadlineSocket::new(stream, accepted_at) else {
        return;
    };
    if let Some(config) = tls {
        let Ok(conn) = rustls::ServerConnection::new(Arc::clone(config)) else {
            return;
        };
        let mut wrapped = rustls::StreamOwned::new(conn, stream);
        let identity_grants: Option<Vec<Grant>> = if let Some(rules) = mtls_rules {
            // WebPkiClientVerifier cryptographically verifies chain, expiry
            // and clientAuth EKU BEFORE peer_certificates() can be trusted.
            while wrapped.conn.is_handshaking() {
                if wrapped.conn.complete_io(&mut wrapped.sock).is_err() {
                    return;
                }
            }
            if wrapped.conn.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3) {
                return;
            }
            // Only the verified end-entity leaf may supply a principal. Use
            // the *same vetted WebPKI X.509 parser as rustls*; never inspect
            // an untrusted CN, client-provided HELLO field, or arbitrary bytes.
            let leaf = match wrapped.conn.peer_certificates().and_then(|chain| chain.first()) {
                Some(leaf) => leaf,
                None => return,
            };
            let parsed = match webpki::EndEntityCert::try_from(leaf) {
                Ok(parsed) => parsed,
                Err(_) => return,
            };
            // #62: Check ALL raw SAN GeneralNames with strict bounded DER
            // traversal. WebPKI's valid_uri_names() deliberately filters
            // undecodable GeneralNames; using that iterator alone could
            // mistake a valid+invalid pair for a single identity.
            let principal = match crate::mtls_san::exact_uri_san(leaf.as_ref()) {
                Some(principal) => principal,
                None => return,
            };
            // Defense in depth: require both independent parsers to agree on
            // the exact, already-verified leaf identity (no CN/normalization).
            let mut identities = parsed.valid_uri_names();
            if identities.next() != Some(principal) || identities.next().is_some() {
                return;
            }
            let grants = match grants_for_mtls_principal(rules, principal) {
                Some(grants) => grants,
                None => return,
            };
            Some(grants)
        } else {
            None
        };
        let closing_after_reply = handle_client_frames(
            &mut wrapped, db, policy, identity_grants.as_deref()
        );
        if closing_after_reply {
            // After an explicit rejection status has been flushed, let the
            // peer consume it and close its side before sending our TLS
            // close_notify and TCP FIN. Network.framework SslStream on macOS
            // can otherwise report EOF instead of delivering the RSWP
            // Unauthorized frame when the TLS peer closes immediately.
            // Never process further requests from a rejected session. Bound
            // this wait to prevent untrusted localhost clients parking a
            // worker indefinitely; production rate limiting is still #39.
            let _ = wrapped.sock.set_read_timeout(Some(Duration::from_millis(750)));
            let mut discarded = [0u8; 1];
            let _ = wrapped.read(&mut discarded);
        }
        if !wrapped.conn.is_handshaking() {
            // Finish the TLS record layer with a proper close_notify alert.
            // An abrupt TCP EOF can cause native Network.framework SslStream
            // on macOS to discard an already-flushed final RSWP status frame.
            // Never close a denied-auth session until both the RSWP frame and
            // TLS close notification have been pushed to the socket.
            wrapped.conn.send_close_notify();
            let _ = wrapped.flush();
        }
    } else {
        let _ = handle_client_frames(&mut stream, db, policy, None);
    }
}

// Returns true only when an explicit final RSWP response requested teardown.
// Invalid/truncated frames and I/O failures do not receive a grace period.
fn handle_client_frames<S: PreAuthTransport>(
    stream: &mut S, db: &Mutex<DocumentStore>, policy: Option<&AuthPolicy>,
    identity_grants: Option<&[Grant]>,
) -> bool {
    let mut greeted = false;
    let mut grants: Option<Vec<Grant>> = None;
    for _ in 0..MAX_REQUESTS_PER_CONNECTION {
        // #72: unauthenticated clients must not reserve the generic
        // 16 MiB frame buffer or pin a worker simply by declaring a huge
        // HELLO body. Once a valid HELLO is authenticated, retain the
        // unchanged 16 MiB CRUD frame limit for the same connection.
        let frame = if greeted {
            if stream.begin_frame_receive().is_err() { return false; }
            let frame = read_frame(stream);
            // Even if the last bytes arrived after 3s, do not process a
            // frame from a peer that exhausted its total ingress budget.
            if stream.finish_frame_receive().is_err() { return false; }
            frame
        } else {
            read_frame_with_limit(stream, MAX_PRE_AUTH_PAYLOAD_BYTES)
        };
        let request = match frame {
            Ok(request) => request,
            Err(_) => return false, // invalid/oversized/truncated frames fail closed
        };
        let was_greeted = greeted;
        if request.kind != Kind::Request {
            return false;
        }
        let (status, payload, close) = if !greeted {
            if request.operation != Operation::Hello {
                (BAD_REQUEST, Vec::new(), true)
            } else if let Some(policy) = policy {
                if let Some(token_grants) = policy.authenticate(&request.payload) {
                    let allowed = match identity_grants {
                        Some(identity) => Grant::intersect(&token_grants, identity),
                        None => token_grants,
                    };
                    if allowed.is_empty() {
                        // A valid token alone cannot authorize any collection;
                        // a valid certificate alone cannot escalate its rights.
                        (UNAUTHORIZED, Vec::new(), true)
                    } else {
                        grants = Some(allowed);
                        greeted = true;
                        (OK, vec![1, 0], false)
                    }
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
        if write_frame(stream, &response).is_err() { return false; }
        // rustls::StreamOwned (and native SslStream peers) may buffer encrypted
        // application records. Never close after an auth denial until status 6
        // has been flushed to the underlying transport. Also flush normal CRUD
        // replies before waiting for another request on persistent connections.
        if stream.flush().is_err() { return false; }
        if !was_greeted && greeted && stream.finish_pre_auth().is_err() {
            return false;
        }
        if close { return true; }
    }
    false
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
