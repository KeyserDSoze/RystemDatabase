import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:net";
import { readFile } from "node:fs/promises";
import { RystemLocalClient } from "../src/client.ts";

// #75: a hypothesis of indefinite TLS handshake trickling was NOT reproduced
// against the unmodified baseline: node:tls already timed out on this partial
// record header. Keep the real TCP adversarial test as a regression, not as a
// claim of a newly implemented absolute timeout.
test("TDD regression #75: incomplete TLS record trickle closes within current socket timeout", {
  timeout: 12000,
}, async () => {
  const sockets = new Set();
  const server = createServer(socket => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
    socket.on("error", () => {});
    socket.write(Buffer.from([0x16]));
    const next = setTimeout(() => {
      if (!socket.destroyed) socket.write(Buffer.from([0x03]));
    }, 1500);
    const last = setTimeout(() => {
      if (!socket.destroyed) socket.write(Buffer.from([0x03]));
    }, 3000);
    socket.on("close", () => { clearTimeout(next); clearTimeout(last); });
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });

  let watchdog;
  try {
    const ca = await readFile(
      new URL("../../../../rystemdb-cli/tests/fixtures/tls/root-ca.pem", import.meta.url),
      "utf8"
    );
    const started = Date.now();
    const attempted = RystemLocalClient.connectTls({
      host: "127.0.0.1", port: server.address().port, serverName: "localhost",
      ca, token: new Uint8Array(32).fill(0x11),
    }).then(
      () => ({ kind: "connected" }),
      error => ({ kind: "rejected", error })
    );
    const observed = await Promise.race([
      attempted,
      new Promise(resolve => {
        watchdog = setTimeout(() => resolve({kind: "watchdog"}), 4600);
      }),
    ]);
    assert.equal(observed.kind, "rejected",
      "TLS slow-trickle listener stayed connected beyond absolute 3s deadline");
    assert.match(String(observed.error), /deadline|timeout/i,
      "timeout must be an explicit client-side handshake deadline");
    assert.ok(Date.now() - started < 4300,
      "incomplete TLS handshake was not terminated within the observed socket timeout");
  } finally {
    clearTimeout(watchdog);
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
  }
});
