//! Security boundary #62: a verified X.509 client leaf must expose exactly
//! one canonical URI GeneralName with no malformed/ambiguous SAN members.
//! This parser never validates certificate signatures; rustls does that first.
const MAX_CERT_DER: usize = 64 * 1024;

/// Strict, bounded DER element reader for *structural validation only*.
///
/// IMPORTANT: This is NOT an X.509 chain/signature/EKU verifier. Rustls must
/// already have accepted the client certificate before this function is
/// called. All lengths use canonical definite DER encoding, full consumption
/// and checked indices; no scan-for-OID or byte-pattern identity extraction.
struct DerReader<'a> {
    remainder: &'a [u8],
}

impl<'a> DerReader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { remainder: bytes } }
    fn is_empty(&self) -> bool { self.remainder.is_empty() }
    fn peek(&self) -> Option<u8> { self.remainder.first().copied() }

    fn take(&mut self, expected: u8) -> Option<&'a [u8]> {
        let (tag, value) = self.next()?;
        (tag == expected).then_some(value)
    }

    fn next(&mut self) -> Option<(u8, &'a [u8])> {
        let original = self.remainder;
        let tag = *original.first()?;
        // Do not accept high-tag-number, EOC, or non-DER indefinite lengths.
        if tag == 0 || tag & 0x1f == 0x1f { return None; }
        let first_len = *original.get(1)?;
        let (len, header) = if first_len < 0x80 {
            (usize::from(first_len), 2usize)
        } else {
            let width = usize::from(first_len & 0x7f);
            if width == 0 || width > 3 { return None; }
            let len_bytes = original.get(2..2 + width)?;
            if len_bytes[0] == 0 { return None; }
            let mut len = 0usize;
            for b in len_bytes {
                len = len.checked_mul(256)?.checked_add(usize::from(*b))?;
            }
            // Long form only for values >= 128 and with minimal width.
            if len < 128 || (width > 1 && len < (1usize << (8 * (width - 1)))) {
                return None;
            }
            (len, 2 + width)
        };
        let end = header.checked_add(len)?;
        let body = original.get(header..end)?;
        self.remainder = original.get(end..)?;
        Some((tag, body))
    }
}

/// Extract an *exact, canonical, single URI SAN* from a leaf already verified
/// by rustls' WebPkiClientVerifier. This deliberately chooses a restricted
/// mTLS identity profile: exactly ONE GeneralName of URI type, no DNS/IP,
/// otherName, empty/malformed/undecodable or repeated SAN extension entries.
///
/// The outer Certificate/TBSCertificate/Extensions are navigated structurally
/// rather than searching for an OID: an unrelated extension, subject/CN or
/// certificate signature cannot be confused with a SAN. The caller must also
/// cross-check the returned value against rustls-webpki's parsed URI iterator.
pub(super) fn exact_uri_san(verified_leaf_der: &[u8]) -> Option<&str> {
    if verified_leaf_der.is_empty() || verified_leaf_der.len() > MAX_CERT_DER {
        return None;
    }
    let mut full = DerReader::new(verified_leaf_der);
    let outer = full.take(0x30)?;
    if !full.is_empty() { return None; }
    let mut certificate = DerReader::new(outer);
    let tbs_der = certificate.take(0x30)?;
    certificate.take(0x30)?; // signatureAlgorithm (crypto verification: rustls)
    certificate.take(0x03)?; // signatureValue
    if !certificate.is_empty() { return None; }

    let mut tbs = DerReader::new(tbs_der);
    // SAN extensions are valid only in explicit v3 TBSCertificate.
    let mut version = DerReader::new(tbs.take(0xa0)?);
    if version.take(0x02)? != [2] || !version.is_empty() { return None; }
    for field in [0x02, 0x30, 0x30, 0x30, 0x30, 0x30] {
        tbs.take(field)?; // serial, signature, issuer, validity, subject, SPKI
    }
    if tbs.peek() == Some(0x81) { tbs.take(0x81)?; } // issuerUniqueID
    if tbs.peek() == Some(0x82) { tbs.take(0x82)?; } // subjectUniqueID
    let mut wrapper = DerReader::new(tbs.take(0xa3)?); // [3] EXPLICIT
    if !tbs.is_empty() { return None; }
    let raw_extensions = wrapper.take(0x30)?;
    if !wrapper.is_empty() { return None; }
    let mut extensions = DerReader::new(raw_extensions);
    let mut san = None;
    let mut count = 0usize;

    while !extensions.is_empty() {
        count += 1;
        if count > 64 { return None; } // resource guard independent of cert size
        let mut ext = DerReader::new(extensions.take(0x30)?);
        let oid = ext.take(0x06)?;
        if oid.is_empty() { return None; }
        if ext.peek() == Some(0x01) {
            // DER BOOLEAN allows only 00 (false) or FF (true).
            if !matches!(ext.take(0x01)?, [0] | [0xff]) { return None; }
        }
        let value = ext.take(0x04)?;
        if !ext.is_empty() { return None; }
        if oid == [0x55, 0x1d, 0x11] { // subjectAltName, 2.5.29.17
            if san.is_some() { return None; } // repeated SAN OID
            let mut encoded = DerReader::new(value);
            let names_der = encoded.take(0x30)?;
            if !encoded.is_empty() { return None; }
            let mut names = DerReader::new(names_der);
            let uri = names.take(0x86)?; // [6] IA5String, IMPLICIT
            // No second URI, DNS, IP, otherName, or malformed GeneralName.
            if !names.is_empty() { return None; }
            let uri = std::str::from_utf8(uri).ok()?;
            let id = uri.strip_prefix("spiffe://rystemdb.local/client/")?;
            if !(1..=64).contains(&id.len()) || !id.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
            }) {
                return None;
            }
            san = Some(uri);
        }
    }
    san
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tlv(tag: u8, bytes: &[u8]) -> Vec<u8> {
        let mut result = vec![tag];
        if bytes.len() < 128 {
            result.push(bytes.len() as u8);
        } else if bytes.len() < 256 {
            result.extend_from_slice(&[0x81, bytes.len() as u8]);
        } else {
            result.extend_from_slice(&[0x82, (bytes.len() >> 8) as u8, bytes.len() as u8]);
        }
        result.extend_from_slice(bytes);
        result
    }
    fn sequence(components: &[Vec<u8>]) -> Vec<u8> {
        tlv(0x30, &components.concat())
    }
    fn uri(bytes: &[u8]) -> Vec<u8> { tlv(0x86, bytes) }
    const WRITER: &[u8] = b"spiffe://rystemdb.local/client/writer";
    const SAN_OID: &[u8] = &[0x55, 0x1d, 0x11];

    fn extension(oid: &[u8], body: &[u8]) -> Vec<u8> {
        sequence(&[tlv(0x06, oid), tlv(0x04, body)])
    }
    fn fake_der_certificate(ext: &[Vec<u8>]) -> Vec<u8> {
        // Syntactically X.509-shaped structure. Not signed and NOT suitable
        // for certificate authentication; only test our bounded DER traversal.
        let tbs = sequence(&[
            tlv(0xa0, &tlv(0x02, &[2])),
            tlv(0x02, &[1]), sequence(&[]), sequence(&[]),
            sequence(&[]), sequence(&[]), sequence(&[]),
            tlv(0xa3, &sequence(ext)),
        ]);
        sequence(&[tbs, sequence(&[]), tlv(0x03, &[0])])
    }
    fn cert_for_names(names: &[Vec<u8>]) -> Vec<u8> {
        fake_der_certificate(&[extension(SAN_OID, &sequence(names))])
    }

    #[test]
    fn red_accepts_single_exact_canonical_uri_from_structural_der() {
        let cert = cert_for_names(&[uri(WRITER)]);
        assert_eq!(exact_uri_san(&cert), Some("spiffe://rystemdb.local/client/writer"));
    }

    #[test]
    fn red_rejects_undecodable_uri_san_even_beside_one_valid_identity() {
        for malformed_uri in [
            vec![0xff], vec![0xc0, 0xaf],
            b"spiffe://rystemdb.local/client/writer%2fadmin".to_vec(),
            b"spiffe://rystemdb.local/client/Writer".to_vec(),
        ] {
            let cert = cert_for_names(&[uri(WRITER), uri(&malformed_uri)]);
            assert!(exact_uri_san(&cert).is_none());
        }
        for extra_name in [
            tlv(0x82, b"localhost"),  // no DNS SAN principals
            tlv(0x87, &[127,0,0,1]),  // no IP SAN principals
            tlv(0x80, b"opaque"),     // no otherName
            tlv(0xa6, WRITER),        // constructed URI tag is forbidden
        ] {
            assert!(exact_uri_san(&cert_for_names(&[uri(WRITER), extra_name])).is_none());
        }
    }

    #[test]
    fn red_rejects_duplicate_extensions_uris_truncated_der_and_trailing_octets() {
        let san = extension(SAN_OID, &sequence(&[uri(WRITER)]));
        assert!(exact_uri_san(&fake_der_certificate(&[san.clone(), san])).is_none());
        assert!(exact_uri_san(&cert_for_names(&[uri(WRITER), uri(WRITER)])).is_none());
        assert!(exact_uri_san(&fake_der_certificate(&[])).is_none());
        let mut wrong = sequence(&[uri(WRITER)]);
        wrong.push(0);
        assert!(exact_uri_san(&fake_der_certificate(&[extension(SAN_OID, &wrong)])).is_none());
        let mut wrong = cert_for_names(&[uri(WRITER)]);
        wrong.pop();
        assert!(exact_uri_san(&wrong).is_none());
        let mut wrong = cert_for_names(&[uri(WRITER)]);
        wrong.push(0);
        assert!(exact_uri_san(&wrong).is_none());
        assert!(exact_uri_san(&vec![0xff; MAX_CERT_DER + 1]).is_none());
    }

    #[test]
    fn red_rejects_noncanonical_der_lengths_and_high_tag_numbers() {
        let mut bad = cert_for_names(&[uri(WRITER)]);
        // Convert root short-form length to a deliberately overlong encoding.
        // If the root uses a long-form length, make it start with a zero.
        if bad[1] & 0x80 == 0 {
            let length = bad[1]; bad.splice(1..2, [0x81, length]);
        } else {
            let n = (bad[1] & 0x7f) as usize;
            bad.splice(1..2, [0x80 | (n as u8 + 1), 0]);
        }
        assert!(exact_uri_san(&bad).is_none());
        assert!(exact_uri_san(&[0x30,0x80,0,0]).is_none());
        assert!(exact_uri_san(&cert_for_names(&[tlv(0x9f, WRITER)])).is_none());
    }
}
