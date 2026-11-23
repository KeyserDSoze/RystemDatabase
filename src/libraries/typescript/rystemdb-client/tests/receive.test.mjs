import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { RystemLocalClient } from "../src/client.ts";
import { encode, decode, HEADER_LEN } from "../src/wire.ts";
import { encodeRdb } from "../src/rdb.ts";

// A synthetic TCP socket exercises the actual receive path deterministically.
// The separate live/mTLS tests continue to use a real Rust subprocess.
class FakeSocket extends EventEmitter {
  destroyed = false;
  sent = [];
  setNoDelay() { return this; }
  setTimeout() { return this; }
  write(data) {
    this.sent.push(Buffer.from(data));
    return true;
  }
  destroy(error) {
    if (this.destroyed) return this;
    this.destroyed = true;
    if (error) this.emit("error", error);
    this.emit("close");
    return this;
  }
}

test("TDD RED #74: fragmented 1 MiB typed GET response must copy frame bytes only O(n)", async () => {
  const socket = new FakeSocket();
  // TS 'private' is a compile-time annotation; the Node 22 type-strip runtime
  // permits injecting a fake transport without altering the SDK public API.
  const client = new RystemLocalClient(socket);
  const blob = Uint8Array.from({length: 1024 * 1024}, (_, i) => i & 0xff);
  const document = new Map([["blob", blob]]);
  const payload = encodeRdb(document);
  const bytes = Buffer.from(encode({
    kind: 2, operation: 17, requestId: 1n, status: 0, payload,
  }));
  const result = client.get("orders", "large");
  assert.equal(decode(socket.sent[0]).operation, 17);
  const originalConcat = Buffer.concat;
  let bytesCopiedByConcat = 0;
  Buffer.concat = function (chunks, length) {
    bytesCopiedByConcat += length ?? chunks.reduce((n, chunk) => n + chunk.length, 0);
    return originalConcat.call(Buffer, chunks, length);
  };
  try {
    // Deliberately split the 24-byte header, then deliver a 1 MiB body in
    // 4 KiB pieces, as can happen with TCP/TLS fragmentation.
    for (let pos = 0; pos < bytes.length; pos += 4096) {
      socket.emit("data", bytes.subarray(pos, Math.min(pos + 4096, bytes.length)));
    }
    const found = await result;
    assert.deepEqual(found, document, "fragmentation must not corrupt typed bytes");
    assert.ok(bytesCopiedByConcat <= bytes.length * 4,
      `reassembled ${bytes.length} bytes via ${bytesCopiedByConcat} Buffer.concat copy-bytes; must be linear`);
  } finally {
    Buffer.concat = originalConcat;
    await client.close();
  }
});

test("TDD RED #74: malformed declared-large RSWP header fails closed before receiving body", async () => {
  const socket = new FakeSocket();
  const client = new RystemLocalClient(socket);
  const header = Buffer.from(encode({
    kind: 2, operation: 17, requestId: 1n, status: 0, payload: new Uint8Array(),
  }));
  assert.equal(header.length, HEADER_LEN);
  header.writeUInt32BE(1024 * 1024, 20);
  header[0] = 0x58; // invalid magic; no body will ever follow
  socket.emit("data", header);
  assert.equal(socket.destroyed, true,
    "client must reject invalid magic immediately, not allocate/wait for a 1 MiB body");
  await client.close();
});
