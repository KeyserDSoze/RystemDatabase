# ADR 0014 — Three-language certificate-verified local TLS 1.3 clients

Status: accepted as experimental local-only SDK integration — 2026-11-21.

Issues [#52](https://github.com/KeyserDSoze/RystemDatabase/issues/52) (TLS SDK), [#53](https://github.com/KeyserDSoze/RystemDatabase/issues/53) (.NET macOS TLS backend), [#54](https://github.com/KeyserDSoze/RystemDatabase/issues/54) (server reply flush); parent [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39) stays OPEN.

## Decision

Retain the single native RSWP v1 framing and typed RDB CRUD logic across explicitly **separate** plaintext and TLS localhost connection modes. Implement TLS1.3 **without any insecure certificate callback** in all three language SDKs:

- Rust: `rustls` with explicit trusted root PEM, SAN/hostname verification and negotiated TLS 1.3 before bearer HELLO.
- .NET 10: `SslStream`, `SslClientAuthenticationOptions.EnabledSslProtocols = Tls13`, `X509ChainPolicy.CustomRootTrust` with caller-specified CA public cert (loaded using `X509CertificateLoader`), built-in SAN verification. No custom `RemoteCertificateValidationCallback`. On macOS enable .NET 10's opt-in Network.framework TLS engine with `AppContext.SetSwitch` **before first TLS use** (CI also sets the process env variable). Never permit TLS1.2.
- Node 22: `node:tls.connect`, `minVersion:TLSv1.3`, `maxVersion:TLSv1.3`, `rejectUnauthorized:true`, explicit CA and DNS/SNI server name. Verify `authorized` and negotiated version before sending the 36-byte authenticated HELLO.

Every SDK rejects nonloopback addresses before opening a TCP connection. Bearer secrets are **never transmitted on unverified TLS**. Authorization remains per collection with explicit read/write grants; unknown tokens and unauthorized collections return RSWP status 6 with an empty body. The Rust server flushes RSWP responses before closing authenticated TLS sessions to avoid lost error frames and client EOF races.

Tests launch the **real native Rust server** as a separate OS process in .NET and Node jobs on Linux, macOS and Windows. Valid cases include nested native binary CRUD, Unicode, concurrent requests and restart persistence. Negative cases include wrong CA, SAN mismatch, expired/not-yet-valid leaf, plaintext-to-TLS downgrade and invalid tokens/collection permissions. Fixture private keys committed in the repository are **public untrusted test fixtures**, not deployment credentials.

## Deliberate non-goals

No production remote bind, mTLS, token expiry/revocation/rotation, certificate pin/rotation policy, audited Windows NTFS ACL security, production rate limiting, released NuGet/npm SDK packages, active cluster routing, secure zeroization guarantees or public exposure is provided. The server and all SDKs still restrict connections to localhost; the existing published `v0.1.2` is a local document CLI only. Complete #39 and the remaining #27/#28 release gates before deploying to other machines.

[Local TLS operation](../docs/LOCAL_TLS.md) | [Native server protocol](../docs/RPC_SERVER.md).
