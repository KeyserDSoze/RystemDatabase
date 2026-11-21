# Experimental local RSWP token authentication and collection ACLs

**Source-only, localhost-only prototype** implemented under [#49](https://github.com/KeyserDSoze/RystemDatabase/issues/49) (native server) and [#50](https://github.com/KeyserDSoze/RystemDatabase/issues/50) (Rust/.NET/Node client interop). Parent [#39](https://github.com/KeyserDSoze/RystemDatabase/issues/39) remains **OPEN**.

**Critical:** this is NOT TLS 1.3, mTLS, a production credential system, or secure remote access. **Tokens travel in plaintext over local TCP** and could be intercepted or replayed by a sufficiently privileged local adversary. Do not expose, tunnel, port-forward or reverse-proxy this endpoint to other machines. A server **always rejects non-loopback** bind addresses, even with \`--auth-file\`. The current published \`v0.1.2\` contains only the local document CLI, NOT this server or these SDK features.

## Development configuration

The existing \`rystemdb rpc-serve --data ... --listen 127.0.0.1:8790\` continues to accept the legacy anonymous HELLO **only on localhost**. For opt-in authenticated local development, provide \`--auth-file\`:

\`\`\`sh
cargo run --locked -p rystemdb-cli -- rpc-serve \
  --data ./rystem-data --listen 127.0.0.1:8790 \
  --auth-file ./private/acl.txt
\`\`\`

The policy is loaded once, at startup. Each nonblank line contains three ASCII fields:

\`\`\`text
<64 lowercase hex characters> <collection> <permissions>
\`\`\`

- Token: exactly **32 bytes of cryptographically random material**, represented as 64 lowercase hex digits in the file. The token bytes—not the hex string—are sent in HELLO.
- Collection: 1–128 ASCII letters, digits, underscores or hyphens; **no wildcard**.
- Permissions: exactly \`r\` for GET, \`w\` for PUT/DELETE or \`rw\` for both. No implicit admin and no anonymous access in authenticated mode. The same token may have separate entries for different collections; duplicate token/collection pairs are rejected.
- Maximum credential file: **64 KiB**, maximum **128 rules**, nonempty and regular file only. Invalid/missing/oversized policies abort startup **before binding the socket or creating the database**.
- On Unix, the credential file must have **no group or other permissions** (for example mode \`0600\`) and must not be a symlink. On Windows, this experimental implementation has **not yet audited or enforced private NTFS ACL ownership/permissions**: configure file ACLs manually and do not treat the Windows prototype as secure storage.

Generate development credentials safely (Unix example; do not copy static example secrets):

\`\`\`sh
mkdir -p private
umask 077
python3 - <<'PY'
from pathlib import Path
from secrets import token_bytes
root = Path("private")
read = token_bytes(32)
write = token_bytes(32)
(root / "read.token").write_bytes(read)
(root / "write.token").write_bytes(write)
(root / "acl.txt").write_text(
    f"{read.hex()} orders r\n{write.hex()} orders w\n",
    encoding="ascii"
)
for file in ("read.token", "write.token", "acl.txt"):
    (root / file).chmod(0o600)
PY
\`\`\`

Keep \`acl.txt\` and \`*.token\` outside version control, build artifacts, backups with broad access and shell history. Pass only the **path** to \`--auth-file\`, not the token. Credential contents must never appear in logs or CLI arguments. Policy changes require a server restart; time-limited tokens, revocation, encrypted secrets and live rotation are **not implemented**.

## HELLO on the wire

RSWP v1 framing remains unchanged: 24-byte header, 16 MiB max body, operation \`HELLO=1\`.

| Mode | HELLO request payload | HELLO success response |
|---|---|---|
| Legacy anonymous local development (no \`--auth-file\`) | \`01 00\` | \`01 00\`, status 0 |
| Authenticated localhost mode | \`01 00 00 20\` + **32 raw token bytes** | \`01 00\`, status 0 |

With \`--auth-file\`, missing, wrong, malformed or incorrect-version HELLO token payload returns **status 6, empty response body**, and then closes the connection. A request before HELLO is rejected. An authenticated session is limited to the collections/operations explicitly listed in its matching token rules: unauthorized GET/PUT/DELETE yields **status 6 with an empty payload**, with no lookup/mutation and no information about document existence. PING requires successful HELLO first.

Secret matching scans the token rules with a fixed-work 32-byte comparison to avoid obvious prefix early-exits, but this is **not formally audited constant-time cryptography** and is not a substitute for secure TLS identity. A compromised token can be replayed until restart/config change.

## Client APIs (source on main)

The clients keep their original anonymous loopback entry points for existing local tests. To use the optional authenticated mode, read the raw 32-byte token from a private file **in the client process**.

**Rust synchronous client:**

\`\`\`rust
use rystemdb_client::LocalClient;
use std::net::SocketAddr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("private/read.token")?;
    let token: [u8; 32] = bytes.try_into()
        .map_err(|_| "expected exactly 32 token bytes")?;
    let addr: SocketAddr = "127.0.0.1:8790".parse()?;
    let mut client = LocalClient::connect_with_token(addr, &token)?;
    client.ping()?;
    let _record = client.get("orders", "u42")?;
    Ok(())
}
\`\`\`

**.NET 10 asynchronous client:**

\`\`\`csharp
using System.Net;
using RystemDatabase.Client;
var token = await File.ReadAllBytesAsync("private/read.token");
await using var db = await RystemLoopbackClient.ConnectWithTokenAsync(
    IPAddress.Loopback, 8790, token);
await db.PingAsync();
var record = await db.GetAsync("orders", "u42");
\`\`\`

**TypeScript/Node 22 asynchronous client:**

\`\`\`typescript
import { readFileSync } from "node:fs";
import { RystemLocalClient } from "./src/client.ts";
const token = new Uint8Array(readFileSync("private/read.token"));
const db = await RystemLocalClient.connect("127.0.0.1", 8790, token);
try {
  await db.ping();
  const record = await db.get("orders", "u42");
} finally {
  await db.close();
}
\`\`\`

The client API does **not** write secrets into process arguments or console output. Bad credentials produce RSWP status 6, exposed as a typed/explicit client error; unauthorized collection operations also return 6. The encrypted remote transport and credential lifecycle are still tracked in #39, and full production SDK hardening in #26–#28.

## TDD and limitations

Rust subprocess tests:
\`\`\`sh
cargo test -p rystemdb-cli --locked --test rpc_auth
cargo test -p rystemdb-cli --locked --test rpc_auth_sdk
\`\`\`

The same CI also builds a real Rust server process in the .NET and Node test jobs on **Linux, macOS and Windows**, then tests missing/wrong token rejection, read-only/write-only collection permissions and typed binary CRUD. The existing default anonymous localhost mode remains tested to avoid a breaking change.

Still **absent**: TLS1.3 and certificate-chain verification, mTLS, secure remote listener, token expiry/revocation/rotation, Windows ACL enforcement, secure secret-in-memory handling, audited constant-time cryptography, replay resistance, per-principal quotas, brute-force/rate limiting, production backpressure and security fuzzing. The opt-in local ACL is **one incremental experimental milestone**, not completion of #39.
