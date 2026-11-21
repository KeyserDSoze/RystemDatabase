# ADR 0015 — Canonical mTLS principal-policy contract (validation-only)

Status: accepted for parser/schema only; TLS client certificate enforcement is pending.
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

## Next steps and tradeoffs

#56 must add audited X.509 parsing and URI SAN extraction *after* rustls verifies
client trust issuer, EKU and certificate validity. Only an exact verified
principal may intersect its collection grants with the authenticated token's
grants; deny requests without both. Continue with negative process tests
(missing, wrong-root, wrong-EKU, expired, unknown/mismatched principal), then
Rust/.NET/Node clients and a revocation/rotation design under #55/#39.
The fixed trust-domain namespace is experimental and will require a versioned
migration rather than silent reassignment if remote identity policy changes.
No remote listener, public mTLS release or production security claim follows.
