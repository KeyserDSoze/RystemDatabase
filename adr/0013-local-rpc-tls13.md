# ADR 0013 — TLS 1.3 certificate-verified Rust RPC transport, loopback-only

Status: accepted as **experimental localhost-only** source milestone, 2026-11-21. Related [#51](https://github.com/KeyserDSoze/RystemDatabase/issues/51) and parent [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

## Decision

Use pinned `rustls 0.23.46` + `rustls-pemfile 2.2.0` (ring crypto provider) for an opt-in TLS1.3-only server mode `rpc-serve --tls-cert ... --tls-key ... --auth-file ...`. **Do not permit TLS1.2, plaintext fallback, external IP or wildcard binding**. Require prevalidated private key/certificate and the already-implemented per-collection token ACL policy before opening any database. Reject incomplete flags, oversized/mismatched PEM, symlinked/unprivileged Unix keys; no Windows ACL validation is claimed.

Refactor server RSWP frame dispatch to generic `Read+Write` while retaining one exclusive DocumentStore, fixed workers, bounded frames, request timeouts and per-session authorization. TLS mode wraps TCP in a `rustls::StreamOwned<ServerConnection, TcpStream>`; legacy plaintext on localhost remains explicit, not an automatic fallback.

Add Rust `LocalClient::connect_tls(addr, server_name, ca_file, raw32_token)`. It rejects non-loopback endpoints, uses a caller-supplied CA root store and validates the server SAN name and chain with rustls, **completes the TLS handshake before sending any bearer token or RSWP data**, and rejects wrong roots/names. No insecure verifier. The client transports binary RSWP unchanged within TLS records. The existing plaintext `connect` APIs remain separate.

Test through actual spawned Rust server processes with an intentionally untrusted publicly committed **test-only** private key, never appropriate for real deployment. Cross-OS Rust CI and strict Clippy must pass. Pin and commit Cargo.lock. The test key fixture is not a secret; do not globally trust its CA.

## Consequences and non-goals

This is **not production-ready remote security**, so *all* non-loopback binds stay denied. The Rust client is synchronous; mTLS, token expiry/revocation/rotation, Windows ACL audit, secure provisioning, quotas/rate limits, performance and cluster identity are separate work under #39, #27, #28. No v0.1.2 release assets were changed.

[Setup and threat model](../docs/LOCAL_TLS.md) | [Local token ACL](../docs/LOCAL_AUTH.md).

## Follow-up — independently implemented cross-language TLS1.3 (#52–54)

The .NET 10 client now uses `SslStream` with explicit custom-root CA and native SAN validation; Node 22 uses `node:tls` with min/max TLSv1.3, `rejectUnauthorized:true`, explicit CA and DNS name. Both enforce TLS1.3 and send bearer HELLO **only after verifying the peer**. The macOS .NET 10 backend needs Network.framework opt-in (#53). The server flushes status 6 before tearing down a denied TLS session (#54). These client transports run actual Rust process tests across Linux/macOS/Windows, rejecting incorrect CA/name, expired and future leaf certs, invalid credentials and plaintext fallback. This does **not** relax the permanent nonloopback bind restriction or close security parent #39. [Usage](../docs/LOCAL_TLS.md).
