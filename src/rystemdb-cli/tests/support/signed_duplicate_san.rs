//! #63: TEST-ONLY construction of a signed X.509 leaf with TWO distinct
//! subjectAltName extensions. OpenSSL's normal -extfile de-duplicates OIDs,
//! so modify a valid ephemeral leaf's *structural* TBS fields, re-sign the
//! complete TBS with that test CA and check its signature independently.
//! Never use this helper for runtime certificate parsing or production PKI.
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

const SAN_OID: [u8; 3] = [0x55, 0x1d, 0x11];
const SHA256_RSA: [u8; 15] = [
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86,
    0xf7, 0x0d, 0x01, 0x01, 0x0b, 0x05, 0x00,
];

pub(super) struct Evidence {
    pub san_extension_count: usize,
    pub signature_valid: bool,
}

struct Field<'a> {
    tag: u8,
    full: &'a [u8],
    body: &'a [u8],
}

fn fields<'a>(bytes: &'a [u8]) -> Vec<Field<'a>> {
    let mut at = 0usize;
    let mut result = Vec::new();
    while at < bytes.len() {
        let start = at;
        let tag = bytes[at];
        let first = *bytes.get(at + 1).expect("truncated test fixture DER header");
        at += 2;
        let length = if first < 0x80 {
            usize::from(first)
        } else {
            let n = usize::from(first & 0x7f);
            assert!((1..=3).contains(&n), "unsupported DER fixture length");
            let mut len = 0usize;
            for byte in bytes.get(at..at + n).expect("truncated DER length") {
                len = (len << 8) | usize::from(*byte);
            }
            at += n;
            len
        };
        let end = at.checked_add(length).expect("DER fixture size overflow");
        let body = bytes.get(at..end).expect("truncated DER fixture body");
        result.push(Field {
            tag, full: &bytes[start..end], body,
        });
        at = end;
    }
    result
}

fn sole(bytes: &[u8], tag: u8) -> Field<'_> {
    let mut parts = fields(bytes);
    assert_eq!(parts.len(), 1, "expected one top-level DER fixture element");
    let part = parts.remove(0);
    assert_eq!(part.tag, tag, "unexpected DER fixture tag");
    part
}

fn encode(tag: u8, bytes: &[u8]) -> Vec<u8> {
    let len = bytes.len();
    let mut out = vec![tag];
    if len < 128 {
        out.push(len as u8);
    } else {
        let big_endian = len.to_be_bytes();
        let first = big_endian.iter().position(|byte| *byte != 0)
            .expect("nonzero length");
        let relevant = &big_endian[first..];
        assert!(relevant.len() <= 3, "oversized test DER element");
        out.push(0x80 | relevant.len() as u8);
        out.extend_from_slice(relevant);
    }
    out.extend_from_slice(bytes);
    out
}

fn is_san_extension(extension: &Field<'_>) -> bool {
    extension.tag == 0x30 && fields(extension.body).first()
        .is_some_and(|oid| oid.tag == 0x06 && oid.body == SAN_OID)
}

fn run_openssl(args: &[&str]) -> String {
    let output = Command::new("openssl").args(args)
        .stdout(Stdio::piped()).stderr(Stdio::null()).output()
        .expect("OpenSSL required to sign ephemeral test certificate");
    assert!(output.status.success(), "test-only openssl {} failed", args[0]);
    String::from_utf8(output.stdout).expect("OpenSSL output should be ASCII")
}

fn utf8_path(path: &Path) -> &str {
    path.to_str().expect("test-only temporary paths are UTF-8")
}

pub(super) fn create(
    temp: &Path, valid_writer_pem: &Path, ca_key: &Path,
    duplicate_pem: &Path,
) -> Evidence {
    let original_der = temp.join("single-san-original.der");
    run_openssl(&[
        "x509", "-in", utf8_path(valid_writer_pem),
        "-outform", "DER", "-out", utf8_path(&original_der),
    ]);
    let bytes = fs::read(&original_der).unwrap();
    let certificate = sole(&bytes, 0x30);
    let cert_parts = fields(certificate.body);
    assert_eq!(cert_parts.len(), 3);
    assert_eq!(cert_parts[0].tag, 0x30); // TBSCertificate
    assert_eq!(cert_parts[1].full, SHA256_RSA);
    assert_eq!(cert_parts[2].tag, 0x03); // BIT STRING signature

    let tbs_parts = fields(cert_parts[0].body);
    assert_eq!(tbs_parts.len(), 8, "expected v3 ephemeral certificate");
    assert_eq!(tbs_parts[0].tag, 0xa0); // version [0] EXPLICIT
    assert_eq!(tbs_parts[2].full, SHA256_RSA);
    assert_eq!(tbs_parts[7].tag, 0xa3); // extensions [3] EXPLICIT
    let original_exts = sole(tbs_parts[7].body, 0x30);
    let extensions = fields(original_exts.body);
    assert_eq!(extensions.iter().filter(|e| is_san_extension(e)).count(), 1,
        "original trusted writer leaf must have exactly one SAN extension");

    // Append a SECOND distinct *extension*, not a second URI entry inside
    // the original SAN sequence. First extension has the canonical writer
    // identity; the added extension names reader, so the policy ambiguity
    // is observable even though both names are syntactically valid.
    let reader = b"spiffe://rystemdb.local/client/reader";
    let additional_san_value = encode(0x30, &encode(0x86, reader));
    let second_san_extension = encode(0x30, &[
        encode(0x06, &SAN_OID),
        encode(0x04, &additional_san_value),
    ].concat());
    let ext_bytes: Vec<u8> = extensions.iter().flat_map(|item| item.full.to_vec())
        .chain(second_san_extension).collect();
    let replaced_extensions = encode(0xa3, &encode(0x30, &ext_bytes));
    let mut tbs_bytes: Vec<u8> = tbs_parts[..7].iter()
        .flat_map(|item| item.full.to_vec()).collect();
    tbs_bytes.extend_from_slice(&replaced_extensions);
    let new_tbs = encode(0x30, &tbs_bytes);

    // The original CA-signed certificate's SHA256/RSA algorithm is retained
    // in BOTH TBS and outer Certificate. Re-sign the changed TBS with the
    // freshly generated test CA private key; a byte mutation without
    // re-signing would only test invalid signatures, not duplicated SAN OIDs.
    let tbs_path = temp.join("double-san-tbs.der");
    let signature_path = temp.join("double-san-signature.bin");
    let ca_public = temp.join("double-san-ca-pub.pem");
    fs::write(&tbs_path, &new_tbs).unwrap();
    run_openssl(&[
        "dgst", "-sha256", "-sign", utf8_path(ca_key),
        "-out", utf8_path(&signature_path), utf8_path(&tbs_path),
    ]);
    run_openssl(&[
        "pkey", "-in", utf8_path(ca_key), "-pubout",
        "-out", utf8_path(&ca_public),
    ]);
    let signature_proof = run_openssl(&[
        "dgst", "-sha256", "-verify", utf8_path(&ca_public),
        "-signature", utf8_path(&signature_path), utf8_path(&tbs_path),
    ]);
    assert!(signature_proof.contains("Verified OK"),
        "independent OpenSSL RSA signature check did not pass");
    let signature = fs::read(&signature_path).unwrap();
    let mut bit_string = vec![0u8]; // 0 unused signature bits
    bit_string.extend_from_slice(&signature);
    let signed_leaf = encode(0x30, &[
        new_tbs, cert_parts[1].full.to_vec(), encode(0x03, &bit_string),
    ].concat());
    let signed_der_path = temp.join("double-san-signed.der");
    fs::write(&signed_der_path, &signed_leaf).unwrap();

    // Independent OpenSSL ASN.1 inspection proves the resulting final,
    // signed leaf contains *two distinct extension OIDs*, not just two
    // names in a single SAN. This must be checked on the finished DER,
    // after reconstructing the signature, before presenting it to Rustls.
    let asn1 = run_openssl(&[
        "asn1parse", "-inform", "DER", "-in", utf8_path(&signed_der_path),
    ]);
    let san_extension_count = asn1.matches("X509v3 Subject Alternative Name").count();
    assert_eq!(san_extension_count, 2,
        "signed fixture must contain exactly two distinct SAN extension OIDs");

    // An actual X.509 PEM decoder must still be able to load the resulting
    // certificate: otherwise an unrelated malformed outer encoding, rather
    // than duplicate SAN extension enforcement, could cause rejection.
    run_openssl(&[
        "x509", "-inform", "DER", "-in", utf8_path(&signed_der_path),
        "-out", utf8_path(duplicate_pem),
    ]);
    run_openssl(&["x509", "-in", utf8_path(duplicate_pem), "-noout", "-subject"]);
    Evidence { san_extension_count, signature_valid: true }
}
