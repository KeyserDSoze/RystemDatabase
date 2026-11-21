import test from "node:test";
import assert from "node:assert/strict";
import { encode, decode, HEADER_LEN, MAX_PAYLOAD_BYTES } from "../src/wire.ts";

const ping = { kind: 1, operation: 2, requestId: 7n, status: 0, payload: new Uint8Array() };
const golden = Uint8Array.from([
    0x52,0x53,0x57,0x50,1,0,1,2,0,0,0,24,
    0,0,0,0,0,0,0,7,0,0,0,0
]);

test("TDD RED: wire ping uses exact Rust octets", () => {
    const bytes = encode(ping);
    assert.deepEqual(bytes, golden);
    assert.equal(bytes.length, HEADER_LEN);
    assert.deepEqual(decode(golden), ping);
});

test("TDD RED: correlation uses exact 64-bit integer (BigInt)", () => {
    const frame = { ...ping, operation: 16, requestId: 18446744073709551615n, payload: Uint8Array.from([0,255,128]) };
    const decoded = decode(encode(frame));
    assert.deepEqual(decoded, frame);
});

test("TDD RED: reject oversized frame before allocating payload", () => {
    const big = Uint8Array.from(golden);
    new DataView(big.buffer).setUint32(20, MAX_PAYLOAD_BYTES + 1, false);
    assert.throws(() => decode(big), /too large|length|limit/i);
});

test("TDD RED: truncation, extra bytes and protocol negotiation fail closed", () => {
    for (let n = 0; n < golden.length; n++) {
        assert.throws(() => decode(golden.subarray(0, n)), /incomplete/i);
    }
    assert.throws(() => decode(Uint8Array.from([...golden, 0])), /trailing/i);
    const wrongVersion = Uint8Array.from(golden);
    wrongVersion[4] = 2;
    assert.throws(() => decode(wrongVersion), /version/i);
});

test("TDD RED: request status must be success and response may report not-found", () => {
    assert.throws(() => encode({ ...ping, status: 1 }), /status/i);
    const response = { ...ping, kind: 2, operation: 17, status: 1, payload: Uint8Array.from([1]) };
    assert.deepEqual(decode(encode(response)), response);
});

test("TDD RED: decoding Node Buffer input always returns canonical Uint8Array wire payload", () => {
  const raw = encode({ ...ping, operation: 16, payload: Uint8Array.from([0,255,128]) });
  const reply = decode(Buffer.from(raw));
  assert.deepEqual(reply.payload, Uint8Array.from([0,255,128]));
  assert.equal(Object.getPrototypeOf(reply.payload), Uint8Array.prototype,
    "Public SDK types must not leak Node Buffer subclasses");
});
