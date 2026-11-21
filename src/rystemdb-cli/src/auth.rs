//! Local-only experimental credential-file authorization.
//!
//! Not TLS or audited cryptography: this policy is intentionally accepted ONLY
//! on loopback and does not make a remote database listener safe. File loading
//! fails closed before the store is opened. No credentials are printed or
//! accepted on argv/environment variables.
use rystemdb_protocol::Operation;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

const MAX_POLICY_BYTES: u64 = 64 * 1024;
const MAX_RULES: usize = 128;
const TOKEN_SIZE: usize = 32;
const INVALID_POLICY: &str = "invalid private RPC auth-file";

#[derive(Clone)]
pub(super) struct Grant {
    collection: String,
    can_read: bool,
    can_write: bool,
}

impl Grant {
    pub(super) fn allows(&self, collection: &str, operation: Operation) -> bool {
        if self.collection != collection {
            return false;
        }
        match operation {
            Operation::Get => self.can_read,
            Operation::Put | Operation::Delete => self.can_write,
            _ => false,
        }
    }
}

/// Bounded policy parsed exactly once at process startup. Credential rotation
/// and revocation without restart are deliberately NOT supported yet.
pub(super) struct AuthPolicy {
    rules: Vec<([u8; TOKEN_SIZE], Grant)>,
}

impl AuthPolicy {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let bytes = read_private_policy(path, INVALID_POLICY)?;
        let contents = std::str::from_utf8(&bytes).map_err(|_| INVALID_POLICY.to_owned())?;
        let mut rules: Vec<([u8; TOKEN_SIZE], Grant)> = Vec::new();
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            if rules.len() >= MAX_RULES {
                return Err(INVALID_POLICY.into());
            }
            let mut parts = line.split_whitespace();
            let token = parts.next().ok_or(INVALID_POLICY)?;
            let collection = parts.next().ok_or(INVALID_POLICY)?;
            let rights = parts.next().ok_or(INVALID_POLICY)?;
            if parts.next().is_some()
                || !valid_collection(collection)
                || !matches!(rights, "r" | "w" | "rw")
            {
                return Err(INVALID_POLICY.into());
            }
            let token = parse_token(token).ok_or(INVALID_POLICY)?;
            if rules.iter().any(|(other, grant)| {
                other == &token && grant.collection == collection
            }) {
                return Err(INVALID_POLICY.into());
            }
            rules.push((token, Grant {
                collection: collection.to_owned(),
                can_read: rights.contains('r'),
                can_write: rights.contains('w'),
            }));
        }
        if rules.is_empty() {
            return Err(INVALID_POLICY.into());
        }
        Ok(Self { rules })
    }

    /// Authenticated HELLO: 01 00 00 20 <32 raw token bytes>.
    /// The token must not appear in command lines or console logs.
    pub(super) fn authenticate(&self, hello: &[u8]) -> Option<Vec<Grant>> {
        if hello.len() != 4 + TOKEN_SIZE || hello[..4] != [1, 0, 0, 32] {
            return None;
        }
        let token: &[u8; TOKEN_SIZE] = hello[4..].try_into().ok()?;
        let mut grants = Vec::new();
        // Scan every rule, and compare all 32 bytes regardless of the matching
        // prefix, avoiding the early-exit behavior of slice ==.
        // This is not a substitute for audited authenticated TLS.
        for (expected, grant) in &self.rules {
            if constant_work_eq(token, expected) {
                grants.push(grant.clone());
            }
        }
        (!grants.is_empty()).then_some(grants)
    }
}

// Both bearer-token and future certificate-principal policies must be
// private, bounded, regular files. Reject symlinks and inode replacement before
// parsing; never include file contents or principal/token values in errors.
fn read_private_policy(path: &Path, invalid: &str) -> Result<Vec<u8>, String> {
    let before = fs::symlink_metadata(path).map_err(|_| invalid.to_owned())?;
    if !before.file_type().is_file() || before.len() > MAX_POLICY_BYTES {
        return Err(invalid.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if before.permissions().mode() & 0o077 != 0 {
            return Err(invalid.into());
        }
    }
    let file = File::open(path).map_err(|_| invalid.to_owned())?;
    let after = file.metadata().map_err(|_| invalid.to_owned())?;
    if !after.is_file() || after.len() > MAX_POLICY_BYTES {
        return Err(invalid.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if before.dev() != after.dev() || before.ino() != after.ino()
            || after.permissions().mode() & 0o077 != 0
        {
            return Err(invalid.into());
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_POLICY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid.to_owned())?;
    if bytes.len() as u64 > MAX_POLICY_BYTES {
        return Err(invalid.into());
    }
    Ok(bytes)
}

const INVALID_MTLS_POLICY: &str = "invalid private mTLS principal policy";
const MTLS_POLICY_VERSION: &str = "rystem-mtls-principal-acl-v1";
const PRINCIPAL_PREFIX: &str = "spiffe://rystemdb.local/client/";

fn valid_mtls_principal(uri: &str) -> bool {
    let Some(name) = uri.strip_prefix(PRINCIPAL_PREFIX) else { return false };
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase()
            || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Parse the *future* mTLS principal-to-collection policy. Parsing deliberately
/// does not authorize a TLS peer: only verified X.509 URI SAN extraction may
/// supply a principal, and the server refuses --mtls-policy until that exists.
fn parse_mtls_principal_policy(contents: &str) -> Result<Vec<(String, Grant)>, String> {
    if !contents.is_ascii() {
        return Err(INVALID_MTLS_POLICY.into());
    }
    let mut lines = contents.lines();
    if lines.next() != Some(MTLS_POLICY_VERSION) {
        return Err(INVALID_MTLS_POLICY.into());
    }
    let mut rules: Vec<(String, Grant)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            return Err(INVALID_MTLS_POLICY.into());
        }
        if rules.len() >= MAX_RULES {
            return Err(INVALID_MTLS_POLICY.into());
        }
        // Exactly three fields and single ASCII space separators. No silent
        // normalizing, wildcard, URI query/fragment, or percent decoding.
        let mut parts = line.split(' ');
        let principal = parts.next().ok_or(INVALID_MTLS_POLICY)?;
        let collection = parts.next().ok_or(INVALID_MTLS_POLICY)?;
        let rights = parts.next().ok_or(INVALID_MTLS_POLICY)?;
        if parts.next().is_some()
            || !valid_mtls_principal(principal)
            || !valid_collection(collection)
            || !matches!(rights, "r" | "w" | "rw")
            || rules.iter().any(|(previous, grant)| {
                previous == principal && grant.collection == collection
            })
        {
            return Err(INVALID_MTLS_POLICY.into());
        }
        rules.push((principal.to_owned(), Grant {
            collection: collection.to_owned(),
            can_read: rights.contains('r'),
            can_write: rights.contains('w'),
        }));
    }
    if rules.is_empty() {
        return Err(INVALID_MTLS_POLICY.into());
    }
    Ok(rules)
}

/// Load and validate an opt-in mTLS policy BEFORE opening any database files.
/// The current source server intentionally refuses to run when this is supplied;
/// parser readiness is not authenticated client-certificate support.
pub(super) fn load_mtls_principal_policy(path: &Path) -> Result<Vec<(String, Grant)>, String> {
    let bytes = read_private_policy(path, INVALID_MTLS_POLICY)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| INVALID_MTLS_POLICY.to_owned())?;
    parse_mtls_principal_policy(text)
}

fn constant_work_eq(a: &[u8; TOKEN_SIZE], b: &[u8; TOKEN_SIZE]) -> bool {
    let mut different = 0u8;
    for i in 0..TOKEN_SIZE {
        different |= a[i] ^ b[i];
    }
    different == 0
}

fn valid_collection(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_token(hex: &str) -> Option<[u8; TOKEN_SIZE]> {
    let encoded = hex.as_bytes();
    if encoded.len() != 2 * TOKEN_SIZE {
        return None;
    }
    let mut bytes = [0u8; TOKEN_SIZE];
    for (i, pair) in encoded.chunks_exact(2).enumerate() {
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        bytes[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_work_comparison_checks_all_bytes() {
        assert!(constant_work_eq(&[5; 32], &[5; 32]));
        let mut altered = [5; 32];
        altered[0] = 4;
        assert!(!constant_work_eq(&[5; 32], &altered));
        altered = [5; 32];
        altered[31] = 4;
        assert!(!constant_work_eq(&[5; 32], &altered));
    }

    #[test]
    fn strict_lowercase_hex_only_and_no_wildcard_collections() {
        assert_eq!(parse_token(&"11".repeat(32)), Some([0x11; 32]));
        assert_eq!(parse_token(&"AA".repeat(32)), None);
        assert_eq!(parse_token("aa"), None);
        assert!(!valid_collection("*"));
        assert!(!valid_collection("../secret"));
    }
    #[test]
    fn mtls_principal_acl_is_canonical_and_exact() {
        let policy = "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/reader orders r\nspiffe://rystemdb.local/client/writer orders rw\n";
        let parsed = parse_mtls_principal_policy(policy).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "spiffe://rystemdb.local/client/reader");
        assert!(parsed[0].1.allows("orders", Operation::Get));
        assert!(!parsed[0].1.allows("orders", Operation::Put));
        assert!(parsed[1].1.allows("orders", Operation::Delete));
        assert!(!parsed[1].1.allows("users", Operation::Get));
    }

    #[test]
    fn mtls_principal_acl_rejects_ambiguous_ids_and_permissions() {
        for id in ["", "*", "Reader", "other/name", "reader%2fadmin",
                   "reader?claim=admin", "reader#admin", "..", "a.b"] {
            assert!(!valid_mtls_principal(
                &format!("spiffe://rystemdb.local/client/{id}")));
        }
        assert!(!valid_mtls_principal("spiffe://evil.local/client/reader"));
        assert!(!valid_mtls_principal(&format!(
            "spiffe://rystemdb.local/client/{}", "a".repeat(65))));
        for body in [
            "rystem-mtls-principal-acl-v2\nspiffe://rystemdb.local/client/a orders r\n",
            "rystem-mtls-principal-acl-v1\n",
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/a * rw\n",
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/a orders read\n",
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/a  orders r\n",
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/a orders rw extra\n",
            "rystem-mtls-principal-acl-v1\n\nspiffe://rystemdb.local/client/a orders r\n",
            "rystem-mtls-principal-acl-v1\nspiffe://rystemdb.local/client/a orders r\nspiffe://rystemdb.local/client/a orders w\n",
        ] {
            assert!(parse_mtls_principal_policy(body).is_err());
        }
    }

}
