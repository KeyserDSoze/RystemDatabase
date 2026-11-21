//! Experimental TLS 1.3-only configuration for **localhost** RSWP.
//! The private key must be protected; this is NOT a remote deployment service.
//! TLS mode additionally requires the existing per-collection auth-file.
use rustls::{ServerConfig, version::TLS13};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

const MAX_PEM_BYTES: u64 = 64 * 1024;
const INVALID_TLS: &str = "invalid TLS server certificate/private-key configuration";

fn read_pem(path: &Path, private: bool) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(path).map_err(|_| INVALID_TLS.to_owned())?;
    if !before.file_type().is_file() || before.len() > MAX_PEM_BYTES {
        return Err(INVALID_TLS.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if private && before.permissions().mode() & 0o077 != 0 {
            return Err(INVALID_TLS.into());
        }
    }
    let file = File::open(path).map_err(|_| INVALID_TLS.to_owned())?;
    let after = file.metadata().map_err(|_| INVALID_TLS.to_owned())?;
    if !after.is_file() || after.len() > MAX_PEM_BYTES {
        return Err(INVALID_TLS.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if before.dev() != after.dev() || before.ino() != after.ino()
            || (private && after.permissions().mode() & 0o077 != 0)
        {
            return Err(INVALID_TLS.into());
        }
    }
    let mut result = Vec::new();
    file.take(MAX_PEM_BYTES + 1)
        .read_to_end(&mut result).map_err(|_| INVALID_TLS.to_owned())?;
    if result.len() as u64 > MAX_PEM_BYTES {
        return Err(INVALID_TLS.into());
    }
    Ok(result)
}

/// Load and verify certificate/key material before creating database files.
pub(super) fn server_config(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, String> {
    let cert_pem = read_pem(cert, false)?;
    let key_pem = read_pem(key, true)?;
    let mut cert_reader = cert_pem.as_slice();
    let chain = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>().map_err(|_| INVALID_TLS.to_owned())?;
    if chain.is_empty() {
        return Err(INVALID_TLS.into());
    }
    let mut key_reader = key_pem.as_slice();
    let private = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|_| INVALID_TLS.to_owned())?.ok_or(INVALID_TLS)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        // Never negotiate TLS1.2, SSL3, plaintext or anonymous cipher suites.
        .with_protocol_versions(&[&TLS13])
        .map_err(|_| INVALID_TLS.to_owned())?
        // RSWP enforces bearer token authorization AFTER TLS negotiation;
        // client-certificate authentication (mTLS) is NOT supported yet.
        .with_no_client_auth()
        .with_single_cert(chain, private)
        .map_err(|_| INVALID_TLS.to_owned())?;
    Ok(Arc::new(config))
}
