# @rystemdb/client (TypeScript / Node.js 22+, experimental localhost-only SDK)

Features completed on source main:
- `src/wire.ts`: strict RSWP v1 24-byte binary framing, unsigned u64 BigInt correlation IDs, bounded 16 MiB payloads.
- `src/crud.ts` and `src/rdb.ts`: strict binary CRUD key and typed nested RDB values, signed i64 BigInt, raw Uint8Array, Map objects, references, arrays/null, UTF-8 golden vectors shared with Rust/.NET.
- **`src/client.ts`: `RystemLocalClient.connect("127.0.0.1", port)`** opens a real TCP socket, negotiates HELLO v1.0, and provides async `ping/put/get/delete/close`. Concurrent requests correlate via request IDs, the socket parser handles TCP segmentation/coalescing and enforces frame/pending bounds. There is no JSON wire conversion.
- Node's native `Buffer` is canonicalized to plain `Uint8Array` in decoded wire payload and typed byte fields (TDD regression #47).
- Real-process CI on **Linux, macOS and Windows** compiles the Rust server and checks nested binary CRUD, Unicode ID, concurrent requests, missing/deleted documents and persistence across server restart.

```sh
cargo build --locked -p rystemdb-cli
RYSTEMDB_BINARY="$(pwd)/target/debug/rystemdb" bash -c \
  'cd src/libraries/typescript/rystemdb-client && npm test'
```

Example:

```typescript
import { RystemLocalClient } from "./src/client.ts";
const db = await RystemLocalClient.connect("127.0.0.1", 8790);
try {
  await db.ping();
  await db.put("users", "u42", new Map([["name", "Ada"], ["age", 34n]]));
  console.log(await db.get("users", "u42"));
} finally {
  await db.close();
}
```

**Important limitations:** localhost-only plaintext or separate verified TLS1.3 (optional local bearer-token ACL; secure remote authentication NOT implemented), no user-provided AbortSignal, connection pools, browser transport, shard routing, automatic idempotent retry, official npm SDK publication, or production resource management. The separate `npm/cli` is a native binary installer wrapper and NOT this SDK. Failed writes may have uncertain commit outcomes. Parent [#28](https://github.com/KeyserDSoze/RystemDatabase/issues/28) remains OPEN; scoped live-connection [#46](https://github.com/KeyserDSoze/RystemDatabase/issues/46).

[Local usage](../../../../docs/LOCAL_CLIENTS.md) | [Typed value format](../../../../docs/RDB_TYPED_PROTOCOL.md)

## Opt-in authenticated localhost HELLO

`RystemLocalClient.connect("127.0.0.1", port, Uint8Array token)` accepts one **32-byte raw** secret and sends `01 00 00 20` + token during HELLO. Wrong/missing credentials and disallowed collection operations raise `RystemRpcError` with `status=6`. Token handling and the server's private `--auth-file` rules are documented in [LOCAL_AUTH.md](../../../../docs/LOCAL_AUTH.md). This is **not encrypted TCP** or a production remote client.


## Verified TLS 1.3 mode (localhost only)

```typescript
import { readFileSync } from "node:fs";
import { RystemLocalClient } from "./src/client.ts";
const db = await RystemLocalClient.connectTls({
  host: "127.0.0.1", port: 8790, serverName: "localhost",
  ca: readFileSync("private/root-ca.pem", "utf8"),
  token: new Uint8Array(readFileSync("private/write.token"))
});
try { await db.ping(); } finally { await db.close(); }
```

`connectTls` uses `node:tls`, **only TLS1.3**, `rejectUnauthorized: true`, explicit CA and standard SAN verification. RSWP bearer HELLO is sent only **after an authorized secureConnect**. Real Rust TLS process tests reject wrong root/name, expired/not-yet-valid certificate, invalid token and plaintext fallback, across Linux/macOS/Windows (#52/#54). The original `connect` remains plaintext local mode; no remote bind, mTLS, pooling or published npm SDK. [LOCAL_TLS.md](../../../../docs/LOCAL_TLS.md).

