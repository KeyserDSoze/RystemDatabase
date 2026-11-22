# ADR 0015 — Canonical mTLS client principal-policy contract

Status: accepted experimental loopback-only source implementation (#57/#58/#60/#61). Production remote identity remains out of scope.
Issues: #57 (this narrow implementation), #56, #55 and #39 (remaining security).

## Context

The experimental loopback RSWP service currently offers certificate-verified
server-side TLS 1.3 and a separate 32-byte bearer-token HELLO with per-collection
ACLs. It does **not** request or verify client TLS certificates. Mapping an
unverified certificate subject/CN string to write permission would be unsafe.

## Decision

Reserve an exact, versioned principal ACL policy grammar:
`rystem-mtls-principal-acl-v1` header followed by exact
`spiffe://rystemdb.local/client/<lowercase-id> <collection> <r|w|rw>` rules.
The parser accepts ASCII only and deliberately rejects percent encoding,
wildcards, duplicated grants, noncanonical identities and ambiguous whitespace.

The same private-file checks used by the existing token ACL are required:
regular file, no symlinks, <=64 KiB, <=128 rules, nonempty; on Unix, no
group/other permissions and inode equality across stat/open. On Windows
private-file ACL audit is a prerequisite for production use.

The optional `rpc-serve --mtls-policy PATH` validates this schema **and then
always errors before binding or creating the database**. A valid policy alone
must never turn on a TLS server that still accepts bearer-only identities.
There is intentionally no new functional auth mode at this milestone.

## Implementation sequence and remaining tradeoffs

#61 added audited X.509 parsing and URI SAN extraction *after* rustls verifies
client trust issuer, EKU and certificate validity. Only an exact verified
principal intersects its collection grants with the authenticated token's
grants; requests without both are denied. Real subprocess tests cover missing
client cert, wrong issuer, wrong EKU, expired/future, unknown/mismatched
principal, disjoint token and read/write rights. Rust SDK now supports source
loopback mTLS; .NET/Node client cert support and a revocation/rotation
design remain separate under #55/#39.
The fixed trust-domain namespace is experimental and will require a versioned
migration rather than silent reassignment if remote identity policy changes.
No remote listener, public mTLS release or production security claim follows.

## Incremental client-certificate verifier gate (#58)

An additional `--mtls-ca` file, only valid together with the explicit
`--mtls-policy`, `--auth-file` and TLS certificate/key, pins a set of client
CA root certificates. Rustls WebPkiClientVerifier requires real X.509 client
authentication on **loopback TLS1.3 only**. At the #58 milestone the listener failed all RSWP
requests, including HELLO, even after certificate verification: certificate
principal URI SAN extraction and ACL intersection were deliberately absent.
That fail-closed interim gate was superseded by #61's verified principal
and token intersection. This prevents a bearer-only token from escalating
a merely TLS-verified client. No private test client keys are committed to the repository in this
milestone. All existing TLS-only and plaintext localhost entry points remain.

## Verified URI SAN principal intersection (#61)

Rustls `WebPkiClientVerifier` enforces client chain, clientAuth EKU and time
before the application can obtain `peer_certificates()`. The server reads
**only the verified leaf**, reparses it using `rustls-webpki 0.103.17`
(which Rustls already depends on), and rejects missing, noncanonical,
unknown or multiple URI SAN principals. The 32-byte HELLO token is separately
authenticated against the private bearer-token ACL; the resulting grants
are **intersected** with the exact canonical URI principal's grants, and
a disjoint intersection fails HELLO. Per-document r/w authorization happens
before any store read or modification. No CN-based, wildcard or token-only
privilege escalation is permitted in this mode.

The certificate CA root, server CA identity and bearer token policy are
configured independently, and all files are read before opening storage.
Live Rust SDK `connect_mtls` tests generate private CA/client identities
per test in temporary directories; these secrets are never committed.

Remain open: production-grade revocation/rotation and audit, additional
strict malformed-name conformance, Windows ACL checks, rate/DoS protection,
non-loopback threat model, .NET/Node client-certificate provisioning (#55/#39).
This ADR does not approve remote listening or publish SDK artifacts.

## Strict single-URI SAN profile and bounded DER guard (#62)

The already verified peer **leaf** is inspected with a bounded, complete
TLV traversal of the DER Certificate/TBSCertificate/extensions path rather
than a blind byte scan. The application requires a v3 TBS certificate, a
single extension OID 2.5.29.17 and exactly one URI [6] GeneralName. This
closed profile intentionally **rejects additional DNS, IP, otherName and
extra URI SANs**, even if one URI is otherwise canonical. The guard rejects
truncated, overlong/indefinite and nonminimal lengths, duplicate SAN OIDs,
invalid UTF-8, unsupported high tag numbers, unexpected trailing data and
DER leafs beyond 64 KiB; at most 64 X.509 extensions are traversed. The
exact name must also match `rustls-webpki`'s independent parsed URI value.

This structural guard does **not** check cryptographic validity, signatures,
issuer trust, time or EKU: only Rustls with WebPkiClientVerifier does, and
it runs **before** this guard. Tests include synthetically malformed DER,
deterministic byte mutations and real CA-signed, OpenSSL-generated ephemeral
bad-client SANs across all three OS. There is no private identity key
committed to Git; the checks are not a formal security audit and do not
approve remote binding or a production PKI.
