//! Security boundary #62: a verified X.509 client leaf must expose exactly
//! one canonical URI GeneralName with no malformed/ambiguous SAN members.
//! This parser never validates certificate signatures; rustls does that first.
const MAX_CERT_DER: usize = 64 * 1024;

#[allow(dead_code)] // RED: wired to the TLS peer after tests demonstrate failure
pub(super) fn exact_uri_san(_verified_leaf_der: &[u8]) -> Option<&str> {
    None // TDD RED: not allowed to silently skip unparseable URI entries
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
