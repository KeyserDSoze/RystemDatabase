//! Experimental Rystem Database **loopback-only synchronous** Rust SDK.
//!
//! Native RSWP v1 HELLO/PING/PUT/GET/DELETE over persistent TCP. No JSON,
//! and no dependency on the CLI process once connected.
//!
//! Security: optional local-token HELLO and verified TLS1.3 (still local-only). External IPs rejected before
//! TCP connection. This is a TDD proof-of-concept, not the future async SDK.
pub use rystemdb_protocol::{
    decode, encode, read_frame, write_frame, Frame, Kind, Operation, ProtocolError,
    HEADER_LEN, MAX_PAYLOAD_BYTES,
};
use rystemdb_core::{decode as decode_document, encode as encode_document, CodecError, Value};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    Protocol(ProtocolError),
    Codec(CodecError),
    RemoteForbidden,
    InvalidKey,
    InvalidDocument,
    Rejected(u16),
    ResponseMismatch,
    RequestIdExhausted,
    TlsConfig,
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "local TCP I/O error: {err}"),
            Self::Protocol(err) => write!(f, "wire protocol error: {err}"),
            Self::Codec(err) => write!(f, "binary document error: {err}"),
            Self::RemoteForbidden => write!(f, "only numeric localhost destinations are supported"),
            Self::InvalidKey => write!(f, "invalid collection or document ID"),
            Self::InvalidDocument => write!(f, "PUT requires an object document"),
            Self::Rejected(status) => write!(f, "server returned RSWP status {status}"),
            Self::ResponseMismatch => write!(f, "server reply did not match the outstanding request"),
            Self::RequestIdExhausted => write!(f, "request ID counter exhausted"),
            Self::TlsConfig => write!(f, "invalid TLS trust or connection configuration"),
        }
    }
}
impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Protocol(err) => Some(err),
            Self::Codec(err) => Some(err),
            _ => None,
        }
    }
}
impl From<io::Error> for ClientError {
    fn from(err: io::Error) -> Self { Self::Io(err) }
}
impl From<ProtocolError> for ClientError {
    fn from(err: ProtocolError) -> Self { Self::Protocol(err) }
}
impl From<CodecError> for ClientError {
    fn from(err: CodecError) -> Self { Self::Codec(err) }
}

/// Single persistent TCP connection to an experimental *localhost only* Rystem
/// server. This is synchronous, not pooled or multi-connection.
enum Transport {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}
impl Read for Transport {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(bytes),
            Self::Tls(stream) => stream.read(bytes),
        }
    }
}
impl Write for Transport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(bytes),
            Self::Tls(stream) => stream.write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// One persistent local binary connection: plaintext developer mode or TLS1.3.
pub struct LocalClient {
    stream: Transport,
    request_id: u64,
}

impl LocalClient {
    /// Connect only to numeric 127.0.0.1/::1, then negotiate RSWP 1.0.
    pub fn connect(addr: SocketAddr) -> Result<Self, ClientError> {
        Self::connect_inner(addr, None)
    }

    /// Authenticate a local RSWP 1.0 session with one 32-byte raw secret.
    /// The token is never placed on argv or logged. This is **not TLS**.
    pub fn connect_with_token(addr: SocketAddr, token: &[u8; 32]) -> Result<Self, ClientError> {
        Self::connect_inner(addr, Some(token))
    }

    fn connect_inner(addr: SocketAddr, token: Option<&[u8; 32]>) -> Result<Self, ClientError> {
        if !addr.ip().is_loopback() {
            return Err(ClientError::RemoteForbidden);
        }
        let stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        stream.set_nodelay(true)?;
        let mut client = Self { stream: Transport::Plain(stream), request_id: 1 };
        client.finish_hello(token)?;
        Ok(client)
    }

    fn finish_hello(&mut self, token: Option<&[u8; 32]>) -> Result<(), ClientError> {
        let mut hello = vec![1, 0];
        if let Some(secret) = token {
            hello.extend_from_slice(&[0, 32]);
            hello.extend_from_slice(secret);
        }
        let frame = self.call(Operation::Hello, hello)?;
        if frame.status != 0 {
            return Err(ClientError::Rejected(frame.status));
        }
        if frame.payload != [1, 0] {
            return Err(ClientError::ResponseMismatch);
        }
        Ok(())
    }

    /// Verified TLS 1.3-only connection to a numeric loopback address.
    /// The server certificate is checked against the explicitly supplied
    /// trust root and name before any RSWP HELLO is sent.
    pub fn connect_tls(
        addr: SocketAddr, server_name: &str, ca_file: &std::path::Path,
        token: &[u8; 32],
    ) -> Result<Self, ClientError> {
        Self::connect_tls_inner(addr, server_name, ca_file, None, token)
    }

    /// Present a separate client certificate and protected key over verified
    /// TLS 1.3, using the standard rustls certificate/key verifier. No secret
    /// is passed in process arguments. This API is loopback-only and does NOT
    /// make #58's deny-all mTLS server into a usable CRUD service until #59.
    ///
    /// On Unix the key file must be regular, not a symlink, and inaccessible
    /// to group/other users. NTFS ACL inspection is not implemented yet (#39).
    pub fn connect_mtls(
        addr: SocketAddr, server_name: &str, ca_file: &std::path::Path,
        client_cert: &std::path::Path, client_key: &std::path::Path,
        token: &[u8; 32],
    ) -> Result<Self, ClientError> {
        Self::connect_tls_inner(addr, server_name, ca_file,
            Some((client_cert, client_key)), token)
    }

    fn connect_tls_inner(
        addr: SocketAddr, server_name: &str, ca_file: &std::path::Path,
        client_identity: Option<(&std::path::Path, &std::path::Path)>,
        token: &[u8; 32],
    ) -> Result<Self, ClientError> {
        // Validate destination before reading any private credentials.
        if !addr.ip().is_loopback() {
            return Err(ClientError::RemoteForbidden);
        }
        let pem = read_tls_pem(ca_file, false)?;
        let mut reader = pem.as_slice();
        let certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>().map_err(|_| ClientError::TlsConfig)?;
        if certs.is_empty() || certs.len() > 8 { return Err(ClientError::TlsConfig); }
        let mut roots = rustls::RootCertStore::empty();
        for cert in certs {
            roots.add(cert).map_err(|_| ClientError::TlsConfig)?;
        }
        let server_name = rustls::pki_types::ServerName::try_from(server_name.to_owned())
            .map_err(|_| ClientError::TlsConfig)?;
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| ClientError::TlsConfig)?
            .with_root_certificates(roots);
        let config = if let Some((cert_path, key_path)) = client_identity {
            let cert_pem = read_tls_pem(cert_path, false)?;
            let private_pem = read_tls_pem(key_path, true)?;
            let mut input = cert_pem.as_slice();
            let chain = rustls_pemfile::certs(&mut input)
                .collect::<Result<Vec<_>, _>>().map_err(|_| ClientError::TlsConfig)?;
            if chain.is_empty() || chain.len() > 8 { return Err(ClientError::TlsConfig); }
            let mut key_input = private_pem.as_slice();
            let key = rustls_pemfile::private_key(&mut key_input)
                .map_err(|_| ClientError::TlsConfig)?.ok_or(ClientError::TlsConfig)?;
            // CertifiedKey::from_der validates key format and that the public
            // key corresponds to the selected leaf certificate. No custom
            // verifier or insecure override is allowed.
            builder.with_client_auth_cert(chain, key)
                .map_err(|_| ClientError::TlsConfig)?
        } else {
            builder.with_no_client_auth()
        };

        let mut socket = TcpStream::connect_timeout(&addr, TIMEOUT)?;
        socket.set_read_timeout(Some(TIMEOUT))?;
        socket.set_write_timeout(Some(TIMEOUT))?;
        socket.set_nodelay(true)?;
        let mut connection = rustls::ClientConnection::new(
            std::sync::Arc::new(config), server_name
        ).map_err(|_| ClientError::TlsConfig)?;
        // Complete chain/hostname verification before sending token or any
        // RSWP bytes; failed TLS never falls back to plaintext.
        while connection.is_handshaking() {
            connection.complete_io(&mut socket)?;
        }
        if connection.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3) {
            return Err(ClientError::TlsConfig);
        }
        let stream = Transport::Tls(Box::new(rustls::StreamOwned::new(connection, socket)));
        let mut client = Self { stream, request_id: 1 };
        client.finish_hello(Some(token))?;
        Ok(client)
    }

    /// Verify the open connection is still responsive; use the same socket.
    pub fn ping(&mut self) -> Result<(), ClientError> {
        let frame = self.call(Operation::Ping, Vec::new())?;
        if frame.status != 0 || !frame.payload.is_empty() {
            return Err(ClientError::Rejected(frame.status));
        }
        Ok(())
    }

    /// Persist one complete nested binary document; an error/timeout may have
    /// an *uncertain* write outcome. Do not blindly retry it.
    pub fn put(&mut self, collection: &str, id: &str, value: &Value) -> Result<(), ClientError> {
        if !matches!(value, Value::Object(_)) {
            return Err(ClientError::InvalidDocument);
        }
        let mut payload = encode_key(collection, id)?;
        payload.extend_from_slice(&encode_document(value)?);
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ClientError::Protocol(ProtocolError::TooLarge));
        }
        let reply = self.call(Operation::Put, payload)?;
        if reply.status != 0 { return Err(ClientError::Rejected(reply.status)); }
        if !reply.payload.is_empty() { return Err(ClientError::ResponseMismatch); }
        Ok(())
    }

    pub fn get(&mut self, collection: &str, id: &str) -> Result<Option<Value>, ClientError> {
        let reply = self.call(Operation::Get, encode_key(collection, id)?)?;
        match reply.status {
            0 => Ok(Some(decode_document(&reply.payload)?)),
            1 if reply.payload.is_empty() => Ok(None),
            _ => Err(ClientError::Rejected(reply.status)),
        }
    }

    pub fn delete(&mut self, collection: &str, id: &str) -> Result<bool, ClientError> {
        let reply = self.call(Operation::Delete, encode_key(collection, id)?)?;
        if !reply.payload.is_empty() { return Err(ClientError::ResponseMismatch); }
        match reply.status {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(ClientError::Rejected(reply.status)),
        }
    }

    fn call(&mut self, op: Operation, payload: Vec<u8>) -> Result<Frame, ClientError> {
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(ClientError::Protocol(ProtocolError::TooLarge));
        }
        let id = self.request_id;
        self.request_id = id.checked_add(1).ok_or(ClientError::RequestIdExhausted)?;
        write_frame(&mut self.stream, &Frame::request(op, id, payload))?;
        let reply = read_frame(&mut self.stream)?;
        if reply.kind != Kind::Response || reply.request_id != id || reply.operation != op {
            return Err(ClientError::ResponseMismatch);
        }
        Ok(reply)
    }
}

/// Read only regular, bounded, non-symlinked TLS PEM files. For private
/// keys, forbid group/other permissions on Unix and ensure the inode did not
/// change between metadata inspection and open. No paths, PEM contents or
/// credential bytes are included in errors. Windows ACL auditing remains #39.
fn read_tls_pem(path: &std::path::Path, private: bool) -> Result<Vec<u8>, ClientError> {
    const MAX_PEM: u64 = 64 * 1024;
    #[cfg(not(unix))]
    let _ = private;
    let before = fs::symlink_metadata(path).map_err(|_| ClientError::TlsConfig)?;
    if !before.file_type().is_file() || before.len() == 0 || before.len() > MAX_PEM {
        return Err(ClientError::TlsConfig);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if private && before.permissions().mode() & 0o077 != 0 {
            return Err(ClientError::TlsConfig);
        }
    }
    let file = File::open(path).map_err(|_| ClientError::TlsConfig)?;
    let after = file.metadata().map_err(|_| ClientError::TlsConfig)?;
    if !after.is_file() || after.len() == 0 || after.len() > MAX_PEM {
        return Err(ClientError::TlsConfig);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if before.dev() != after.dev() || before.ino() != after.ino()
            || (private && after.permissions().mode() & 0o077 != 0)
        {
            return Err(ClientError::TlsConfig);
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_PEM + 1).read_to_end(&mut bytes).map_err(|_| ClientError::TlsConfig)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_PEM {
        return Err(ClientError::TlsConfig);
    }
    Ok(bytes)
}

/// Use the exact shared RSWP v1 key codec. No duplicate validation,
/// UTF-8 encoding or byte-length handling in the client.
fn encode_key(collection: &str, id: &str) -> Result<Vec<u8>, ClientError> {
    rystemdb_protocol::crud::encode_key(collection, id)
        .map_err(|_| ClientError::InvalidKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_octets_are_identical_to_server_contract() {
        assert_eq!(encode_key("users", "u42").unwrap(),
            b"\x00\x05users\x00\x03u42");
        for (collection, id) in [
            ("", "one"),
            ("bad/path", "one"),
            ("users", ""),
            ("users", "bad\nid"),
        ] {
            assert!(matches!(encode_key(collection, id), Err(ClientError::InvalidKey)));
        }
    }

    #[test]
    fn mtls_client_refuses_nonloopback_before_reading_any_credential_file() {
        let missing = std::path::Path::new("missing-client-mtls-credential");
        for addr in ["0.0.0.0:8790", "198.51.100.10:8790", "[2001:db8::1]:8790"] {
            let result = LocalClient::connect_mtls(
                addr.parse().unwrap(), "localhost", missing, missing, missing, &[0; 32]
            );
            assert!(matches!(result, Err(ClientError::RemoteForbidden)));
        }
    }

    #[test]
    fn connection_is_explicitly_localhost_only() {
        for addr in ["0.0.0.0:8790", "198.51.100.10:8790", "[2001:db8::1]:8790"] {
            assert!(matches!(LocalClient::connect(addr.parse().unwrap()), Err(ClientError::RemoteForbidden)));
        }
    }
}
