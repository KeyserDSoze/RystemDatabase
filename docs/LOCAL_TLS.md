# TLS 1.3 on loopback — experimental Rust, .NET and Node client/server preview

Implemented in source on `main`: Rust TLS [#51](https://github.com/KeyserDSoze/RystemDatabase/issues/51), .NET/Node TLS [#52](https://github.com/KeyserDSoze/RystemDatabase/issues/52), and macOS TLS backend fix [#53](https://github.com/KeyserDSoze/RystemDatabase/issues/53), under still-open security parent [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39).

**This is real certificate-verified TLS 1.3 but NOT a remotely deployable database service.** The server and Rust client still reject every non-loopback address; production mTLS identity rotation/revocation, Windows ACL audit, rate limiting, cluster authorization and public network access are not implemented. The published `v0.1.2` binaries do not contain `rpc-serve` or TLS.

## Source build and configuration

```bash
cargo build --locked -p rystemdb-cli

./target/debug/rystemdb rpc-serve \
  --data ./rystem-data \
  --listen 127.0.0.1:8790 \
  --auth-file ./private/acl.txt \
  --tls-cert ./private/server-cert.pem \
  --tls-key ./private/server-key.pem
```

TLS and authentication are **opt-in together**: `--tls-cert` and `--tls-key` must both be provided, and TLS mode **requires** `--auth-file` with explicit collection read/write grants. Missing, malformed, oversized or mismatched key/cert, absent ACL, loose Unix key permissions and symlinked PEM keys fail at startup, before creating storage files. The private key is only read from an explicit file path (not from process arguments or logs). The certificate chain can be publicly readable. The max PEM length is 64 KiB per file.

Use a **privately generated CA and server key** for development. The server cert must contain a Subject Alternative Name matching the name the client verifies (for example `DNS:localhost`); its private key must match the certificate. Protect key and ACL files with mode `0600` on Unix and restrictive NTFS ACLs on Windows. The checked-in `src/rystemdb-cli/tests/fixtures/tls` key is **PUBLIC and for automated tests only**, never for actual use. Do not install the test root certificate into a global trust store.

## Rust SDK: certificate chain and server name verification

```rust
use rystemdb_client::LocalClient;
use std::{fs, net::SocketAddr, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read("private/write.token")?;
    let token: [u8; 32] = bytes.try_into().map_err(|_| "wrong token size")?;
    let addr: SocketAddr = "127.0.0.1:8790".parse()?;

    // Connect only to numeric localhost, verify cert chain against the
    // supplied CA, and verify SAN DNS name "localhost".
    // The bearer token is sent ONLY AFTER the verified TLS 1.3 handshake.
    let mut db = LocalClient::connect_tls(
        addr, "localhost", Path::new("private/root-ca.pem"), &token,
    )?;
    db.ping()?;
    Ok(())
}
```

The client uses `rustls 0.23.46`, the ring provider, **only TLS 1.3**, a local CA trust store and strict SAN/name validation. There is no insecure custom verifier or plaintext fallback. A wrong root, wrong server name or TLS handshake failure is an error **before** token transmission. Within the authenticated TLS stream, native RSWP v1 framing, typed binary CRUD and collection ACL status codes remain unchanged.

`LocalClient::connect` and `LocalClient::connect_with_token` remain **plaintext localhost-only developer prototypes** and cannot communicate with a TLS-only listener. The .NET/Node SDKs now have **separate, verified TLS 1.3 entry points** described below. Their legacy `ConnectAsync`/`ConnectWithTokenAsync`/`connect` still mean explicit plaintext localhost mode; no automatic or silent fallback.

## Security and negative tests

```bash
cargo test -p rystemdb-cli --test rpc_tls --locked
cargo test --workspace --locked --all-targets
cargo clippy --workspace --locked --all-targets -- -D warnings
```

The real-process integration suite starts the executable with `--listen 127.0.0.1:0` to avoid ephemeral port races and verifies TLS 1.3 Rust-client HELLO/PING/native typed CRUD, wrong trusted CA, wrong hostname, plaintext RSWP against the TLS port, missing/incorrect bearer tokens, malformed/mismatched key/certificate, insecure TLS config, and non-loopback bind denial. The same Cargo suite executes on Linux, macOS and Windows in CI.

**Important remaining risks:** Publicly exposed bind remains prohibited even with TLS; source-only, local mTLS principal/token ACLs are experimental and lack operational revocation/rotation, no cluster admission control, no certificate expiry/rotation/revocation workflow, no Windows ACL validation, no per-user rate limits, no full parser fuzzing or comprehensive remote threat model. Compromised local credentials and on-disk files are outside this transport layer's protection. Complete [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39) before opening to other machines. [RSWP protocol](WIRE_PROTOCOL.md) | [Local bearer-token policy](LOCAL_AUTH.md) | [Local SDK guide](LOCAL_CLIENTS.md).


## C#/.NET 10: verified SslStream, explicit root CA and SAN

```csharp
using System.Net;
using RystemDatabase.Client;

byte[] secret = await File.ReadAllBytesAsync("private/write.token");
// On macOS, enable .NET 10's Network.framework-backed TLS1.3 BEFORE
// the first SslStream in your application (the SDK also opts in).
if (OperatingSystem.IsMacOS())
    AppContext.SetSwitch("System.Net.Security.UseNetworkFramework", true);
await using var db = await RystemLoopbackClient.ConnectTlsAsync(
    IPAddress.Loopback, 8790, "localhost", "private/root-ca.pem", secret);
await db.PingAsync();
```

The .NET client uses `SslStream` and `SslClientAuthenticationOptions` with `EnabledSslProtocols = Tls13`, explicit `X509ChainPolicy` with `CustomRootTrust`, and the **standard operating-system certificate and DNS SAN validator** (no custom bypass callback). The SDK requires `SslProtocol == Tls13` and `IsAuthenticated && IsEncrypted` before sending bearer token HELLO. Public CA PEM is loaded with `X509CertificateLoader.LoadCertificateFromFile`, which does not require the CA's private key.

**macOS .NET 10:** legacy SecureTransport does not support TLS1.3; the SDK opts into Apple's Network.framework using `AppContext.SetSwitch("System.Net.Security.UseNetworkFramework", true)`. If your application already initialized SslStream, set this switch at **process startup**, or use environment variable `DOTNET_SYSTEM_NET_SECURITY_USENETWORKFRAMEWORK=1`; CI sets this for macOS. No TLS1.2 fallback is permitted. This behavior is tracked and tested in #53.

The test fixture CA intentionally has no CRL. The client therefore sets `RevocationMode = NoCheck` **only within this experimental localhost prototype**. Proper revocation, pinning/rotation policies and production-grade mTLS are remaining #39 requirements.

## Node.js 22+: certificate-verified node:tls client

```typescript
import { readFileSync } from "node:fs";
import { RystemLocalClient } from "./src/client.ts";

const db = await RystemLocalClient.connectTls({
  host: "127.0.0.1",
  port: 8790,
  serverName: "localhost",
  ca: readFileSync("private/root-ca.pem", "utf8"),
  token: new Uint8Array(readFileSync("private/write.token")),
});
try {
  await db.ping();
  await db.put("orders", "o42", new Map([
    ["paid", true], ["amountInCents", 1234n],
  ]));
} finally {
  await db.close();
}
```

Node uses `node:tls`, `minVersion: "TLSv1.3"`, `maxVersion: "TLSv1.3"`, `rejectUnauthorized: true`, explicit CA, and the `serverName` SNI/DNS SAN hostname. It sends RSWP HELLO with raw token **only after** an authorized `secureConnect` and confirms the negotiated protocol is `TLSv1.3`. A wrong CA, hostname, expired/not-yet-valid certificate or failed TLS handshake aborts without exposing the token; there is no custom verifier or plaintext fallback.

## Cross-language process-level security regressions

`TlsContract.cs` and `tests/tls.test.mjs` launch the **real Rust executable** on Linux, macOS and Windows and verify successful TLS HELLO/PING, nested typed binary CRUD (bytes, references, i64, Unicode), concurrent requests, collection read/write denial, wrong CA/SAN, malformed credential, plaintext-to-TLS refusal and persistence across separate server processes. Additional **public** expiry fixtures ensure both languages reject server certificates whose notBefore/notAfter window excludes the current date, even when CA and SAN match. An RSWP Unauthorized response is explicitly flushed before the Rust server closes its TLS stream (#54).

**These are not production remote TLS SDKs.** The server and all client APIs still reject non-loopback addresses. Source-only Rust mTLS exists on loopback, but there is no .NET/Node mTLS client-certificate provisioning, centralized identities, certificate/key rotation, revocation workflow, secure connection pools, released NuGet/npm SDK packages or service discovery. [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39) remains open.

## mTLS principal policy and verified URI SAN authorization (#57/#58/#61)

An explicit versioned client-principal policy is recognized by the CLI. With
only `--mtls-policy`, startup still refuses safely. Supplying BOTH
`--mtls-policy PRIVATE_PATH` and `--mtls-ca CLIENT_ROOT_PEM`, together with
`--auth-file` and `--tls-cert/--tls-key`, opts into an **experimental TLS1.3
client-certificate authenticated loopback service** backed by Rustls
WebPkiClientVerifier. The client certificate chain, clientAuth EKU and
validity are verified *before* extracting a URI SAN from the authenticated
leaf certificate. Only the exact, canonical, single URI SAN with a matching
principal policy, AND a separately authenticated 32-byte HELLO bearer token,
can grant RSWP collection access. Granted operations are the intersection
of the two ACLs; neither certificate nor token grants access by itself.
This is source-only, not production remote mTLS or released SDK software.

The first line must be `rystem-mtls-principal-acl-v1`. Each nonempty subsequent
line must have exactly three ASCII fields separated by one space:

```text
rystem-mtls-principal-acl-v1
spiffe://rystemdb.local/client/reader orders r
spiffe://rystemdb.local/client/writer orders rw
```

The URI is an **exact-match SPIFFE-style URI SAN identity**, not a CN,
HTTP credential, or human-readable display name. The fixed experimental prefix
is `spiffe://rystemdb.local/client/` followed by 1–64 lowercase ASCII letters,
digits, underscores or hyphens. Percent encoding, extra path segments, queries,
fragments and wildcards are rejected; there is no identity normalization. The
collection syntax and `r`/`w`/`rw` semantics mirror the local token policy.
At most 128 rules in a nonempty, regular, UTF-8 ASCII file of at most 64 KiB.
Duplicate (principal, collection) rules and insecure Unix permissions/symlinks
are rejected without echoing principals in errors.

The server reads only the **verified peer end-entity leaf** obtained from
Rustls after completing the mandatory TLS1.3 client-certificate handshake.
It uses the pre-existing `rustls-webpki` X.509 parser to extract URI SAN
values, requires exactly one canonical URI principal and exact rules
(no case folding or percent decoding), then computes the strict
**certificate principal ACL ∩ bearer-token ACL** during HELLO. Unknown or
ambiguous principals fail closed. No document read/write is performed if
either credential or the required collection right is absent.

This localhost-only experimental feature does **not** enable non-loopback
binding, real certificate revocation/rotation, audited Windows NTFS ACLs,
public service discovery or production certificate lifecycle management.
[#55](https://github.com/KeyserDSoze/RystemDatabase/issues/55) and
[#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39)
remain open.

## Denied TLS HELLO teardown regression (#54)

A macOS .NET 10 Network.framework `SslStream` regression showed that sending
a valid status-6 RSWP frame followed immediately by TLS `close_notify`/TCP
FIN can sometimes be observed as EOF instead of an application response.
The server explicitly flushes the RSWP frame and, on a *final response that
requests session closure*, now **waits at most 750 ms for the peer to consume
the response and close its side** before sending its TLS close notification.
The waiting bytes are discarded, never parsed as a second request. Other
session types and the plaintext local prototype retain their behavior.
The bound limits resource occupancy by clients that never close; production
rate limiting and security DoS review remain open under #39. The real-process
Rust/.NET/Node rejection tests must observe actual RSWP status 6, never
interpret EOF as a successful authorization denial.

### Rustls client certificate principal-bound ACLs (#58/#61, localhost only)

```sh
rystemdb rpc-serve --data ./rystem-data --listen 127.0.0.1:8790 \
  --auth-file ./private/token-acl \
  --tls-cert ./private/server-cert.pem --tls-key ./private/server-key.pem \
  --mtls-policy ./private/client-principals \
  --mtls-ca ./private/client-root-ca.pem
```

This listener requires a certificate issued by the **explicit, independently
selected** client CA. Rustls checks trust, TLS1.3 and client-auth
certificate validity/EKU before RSWP. A verified leaf with exactly one
canonical URI SAN mapped to the private principal ACL can authenticate
**only with a matching HELLO token**. A `reader orders r` certificate
principal cannot use a token with `orders rw` to write or delete. Unauthorized
document requests return status 6 without revealing existence or data; an
unknown principal cannot complete RSWP HELLO. There is no token-only,
CN-based, plaintext or non-loopback fallback. Legacy TLS localhost without
mTLS flags is unchanged. This is NOT an operational production service.

### Rust SDK client certificate presentation (#60)

The Rust SDK offers `LocalClient::connect_mtls` with an explicit server CA
file, client certificate and private-key paths plus 32-byte token. It validates
TLS1.3 server identity and securely loads bounded PEM files; private key
permissions are checked on Unix. A real Rustls test peer verifies an ephemeral
client certificate signed by a separately generated test CA, then exchanges
RSWP HELLO/PING. No long-term or production private client keys are in git.

The real Rust CLI now authorizes `connect_mtls` only after mandatory
cryptographic certificate verification, single canonical URI SAN identity
mapping and a successful token HELLO, with intersection of both ACLs (#61).
`cargo test --locked -p rystemdb-cli --test rpc_mtls_identity` covers real
process typed CRUD and extensive negative identities using ephemeral OpenSSL
test-only certificates, plus `rpc_mtls_sdk` for transport and invalid tokens.
Legacy `connect_tls` remains functional; no remote addresses or automatic
fallback are allowed.
