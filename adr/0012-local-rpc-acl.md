# ADR 0012 — Opt-in localhost credential HELLO and per-collection ACL

Status: accepted as **experimental and local-only**, 2026-11-21. Not production authentication.

Related [#49](https://github.com/KeyserDSoze/RystemDatabase/issues/49), [#50](https://github.com/KeyserDSoze/RystemDatabase/issues/50), parent [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

## Context

The native Rust RPC service and Rust/.NET/Node SDKs had real binary CRUD over localhost but no way to restrict access to specific collections or separate readers from writers. The server must **not** accept external clients before TLS identity verification and authenticated authorization are complete.

## Decision

- Keep the non-loopback listener prohibition in all configurations; source-only localhost development mode is not remotely secure.
- An explicit \`--auth-file PRIVATE_PATH\` enables a **deny-by-default** local session mode. Omit it to retain the existing backward-compatible anonymous localhost experimental mode.
- File syntax: 64 lowercase hex characters (32-byte raw secret), exact ASCII collection name and \`r\`, \`w\` or \`rw\`; no wildcard, duplicate token/collection grant or implicit admin. Maximum 64 KiB and 128 grants. Fail closed for invalid/missing/oversized file **before binding or opening data**. Unix rejects symlinks, world/group permissions and file swaps detected by dev/inode checks. Windows NTFS ACL hardening is a distinct outstanding security gate.
- HELLO v1.0 payload \`01 00 00 20 <32 raw token bytes>\` is required in auth mode; wrong/missing tokens return status \`6\` and close, no information-bearing error. Legacy \`01 00\` only works when auth mode is off.
- Store session grants **per TCP connection**. Validate collection and operation at the dispatch boundary and deny without touching document storage. Reads require \`r\`, PUT/DELETE require \`w\`; forbidden and missing collections use the same status 6 regardless of existence.
- Implement the matching HELLO form in local Rust \`LocalClient\`, .NET \`RystemLoopbackClient\` and Node \`RystemLocalClient\` without exposing tokens in argv, command logs or JSON. Preserve the earlier anonymous local APIs. Run native Rust subprocess and full client interop tests on Linux/macOS/Windows.
- A fixed-work byte comparison avoids obvious early-exit token prefix matches. It is **not** audited cryptographic constant-time behavior.

## Limitations / threat model

**The token travels in plaintext over local TCP**, can be intercepted/replayed by a privileged local attacker, and is a long-lived bearer secret. There is no TLS/mTLS, certificate validation, nonce, expiry, remote identity, rotation or revocation without restart; Windows private-file ACL validation is absent. The legacy local mode remains anonymous by default. Policies do not grant OS-level filesystem isolation. An attacker with direct write access to the database folder can bypass this network ACL.

This milestone is deliberately incapable of binding non-loopback and must not be presented as an authenticated remote deployment. Parent #39 remains OPEN for TLS1.3 authenticated transport, secure credentials and production-grade authorization/rate limits.

[Operational example](../docs/LOCAL_AUTH.md), [wire protocol](../docs/WIRE_PROTOCOL.md), [local SDK guide](../docs/LOCAL_CLIENTS.md).
