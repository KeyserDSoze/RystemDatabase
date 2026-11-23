import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { RystemLocalClient } from "../src/client.ts";
import { encode, decode, MAX_PAYLOAD_BYTES, HEADER_LEN } from "../src/wire.ts";

// A controlled writable socket tests the real SDK write path without
// allocating gigabytes or relying on nondeterministic kernel TCP buffers.
class StalledSocket extends EventEmitter {
  destroyed = false;
  writableLength = 0;
  sent = [];
  stallFirst = false;
  throwOnWrite = false;
  setNoDelay() { return this; }
  setTimeout() { return this; }
  write(bytes) {
    if (this.throwOnWrite) throw new Error("synthetic synchronous write error");
    this.sent.push(Buffer.from(bytes));
    this.writableLength += bytes.length;
    return !(this.stallFirst && this.sent.length === 1);
  }
  drained() {
    this.writableLength = 0;
    this.emit("drain");
  }
  destroy(error) {
    if (this.destroyed) return this;
    this.destroyed = true;
    if (error) this.emit("error", error);
    this.emit("close");
    return this;
  }
}

function success(id) {
  return Buffer.from(encode({
    kind: 2, operation: 2, requestId: id,
    status: 0, payload: new Uint8Array(),
  }));
}

test("TDD RED #76: false socket.write blocks further calls until drain, without sending them", async () => {
  const socket = new StalledSocket();
  socket.stallFirst = true;
  const client = new RystemLocalClient(socket);
  let first;
  let refused;
  let afterDrain;
  try {
    first = client.ping();
    first.catch(() => {});
    refused = client.ping();
    refused.catch(() => {});
    assert.equal(socket.sent.length, 1,
      "a backpressured socket must NOT receive a second write before 'drain'");
    await assert.rejects(refused, /backpressure|write buffer|queue full/i,
      "unsent call should return an explicit local backpressure error");
    socket.drained();
    afterDrain = client.ping();
    afterDrain.catch(() => {});
    assert.equal(socket.sent.length, 2,
      "the same socket must resume legitimate requests after drain");
    const ids = socket.sent.map(raw => decode(raw).requestId);
    socket.emit("data", Buffer.concat([success(ids[0]), success(ids[1])]));
    await Promise.all([first, afterDrain]);
    assert.equal(socket.destroyed, false);
  } finally {
    if (first) first.catch(() => {});
    if (refused) refused.catch(() => {});
    if (afterDrain) afterDrain.catch(() => {});
    await client.close();
  }
});

test("TDD RED #76: writableLength near 16 MiB rejects request before any write", async () => {
  const socket = new StalledSocket();
  socket.writableLength = MAX_PAYLOAD_BYTES + HEADER_LEN - 2;
  const client = new RystemLocalClient(socket);
  try {
    const pending = client.ping();
    pending.catch(() => {});
    assert.equal(socket.sent.length, 0,
      "socket write-queue budget must reject before sending bytes");
    await assert.rejects(pending, /backpressure|write buffer|queue full/i);
    assert.equal(socket.destroyed, false,
      "local unsent backpressure must not poison an otherwise healthy connection");
  } finally {
    await client.close();
  }
});

test("TDD RED #76: synchronous write failure clears pending promise and closes transport", async () => {
  const socket = new StalledSocket();
  socket.throwOnWrite = true;
  const client = new RystemLocalClient(socket);
  try {
    await assert.rejects(client.ping(), /write|synthetic/i);
    assert.equal(socket.destroyed, true, "write failure must close an unsafe transport");
    assert.equal(client.pending.size, 0,
      "write failure must not retain a pending request and its timer");
  } finally {
    await client.close();
  }
});
