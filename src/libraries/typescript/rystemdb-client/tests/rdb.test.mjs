import test from "node:test";
import assert from "node:assert/strict";
import { RdbReference, encodeRdb, decodeRdb } from "../src/rdb.ts";

// Frozen literal, hand-authored from Rust rystemdb-core v1 binary format.
// { "a": [null, true, -1], "b": Bytes(00 ff 80), "r": Ref("users","u42") }
const GOLDEN = Uint8Array.from(Buffer.from(
  "52444200010800000003" +
  "00000001610700000003000203ffffffffffffffff" +
  "0000000162060000000300ff80" +
  "00000001720900000005757365727300000003753432", "hex"
));
const SAMPLE = new Map([
  ["a", [null,true,-1n]],
  ["b", Uint8Array.from([0,255,128])],
  ["r", new RdbReference("users","u42")],
]);
test("TDD RED: TS typed nested document matches Rust golden bytes", () => {
  assert.deepEqual(encodeRdb(SAMPLE),GOLDEN);
  assert.deepEqual(decodeRdb(GOLDEN),SAMPLE);
});
test("TDD RED: UTF-8 object ordering, missing versus null, signed i64 and float", () => {
  const input = new Map([
    ["😀","astral"],["\uE000","bmp"],["nullable",null],
    ["min",-9223372036854775808n],["max",9223372036854775807n],
    ["float",1.25],["string","Roma 🏛"],
  ]);
  const round = decodeRdb(encodeRdb(input));
  assert.deepEqual(round,input);
  assert.equal(round.has("missing"),false);
  assert.equal(round.get("nullable"),null);
  assert.throws(()=>encodeRdb(new Map([["x",9223372036854775808n]])),/i64|integer|range/i);
  assert.throws(()=>encodeRdb(new Map([["x",Number.NaN]])),/finite/i);
  assert.throws(()=>encodeRdb(new Map([["x",Number.POSITIVE_INFINITY]])),/finite/i);
});
test("TDD RED: binary decoder rejects truncation and trailing data", () => {
  for(let i=0;i<GOLDEN.length;i++) {
    assert.throws(()=>decodeRdb(GOLDEN.subarray(0,i)),/incomplete|truncat|end/i);
  }
  assert.throws(()=>decodeRdb(Uint8Array.from([...GOLDEN,0])),/trailing/i);
});
test("TDD RED: decoder rejects malformed UTF-8, unknown tags and duplicate keys", () => {
  const wrongMagic=Uint8Array.from(GOLDEN); wrongMagic[0]=0;
  assert.throws(()=>decodeRdb(wrongMagic),/magic/i);
  const wrongVersion=Uint8Array.from(GOLDEN); wrongVersion[4]=2;
  assert.throws(()=>decodeRdb(wrongVersion),/version/i);
  const unknown=Uint8Array.from([82,68,66,0,1,255]);
  assert.throws(()=>decodeRdb(unknown),/tag/i);
  const wrongUtf8=Uint8Array.from([82,68,66,0,1,5,0,0,0,1,255]);
  assert.throws(()=>decodeRdb(wrongUtf8),/utf/i);
  const duplicate=Uint8Array.from(Buffer.from(
    "52444200010800000002" + "000000017800" + "000000017802", "hex"));
  assert.throws(()=>decodeRdb(duplicate),/duplicate/i);
});
test("TDD RED: decoding enforces nesting depth and rejects malformed references", () => {
  let nested = null;
  for(let i=0;i<65;i++) nested=[nested];
  assert.throws(()=>encodeRdb(nested),/depth|deep/i);
  assert.throws(()=>encodeRdb(new RdbReference("","42")),/reference/i);
  const invalidRef=Uint8Array.from(Buffer.from("5244420001090000000000000003313233","hex"));
  assert.throws(()=>decodeRdb(invalidRef),/reference/i);
});

test("TDD RED: typed RDB retains U+FEFF at start of object keys, values and references", () => {
  const data = new Map([
    ["\uFEFFname", "\uFEFFvalue"],
    ["reference", new RdbReference("users", "\uFEFFid")],
  ]);
  const encoded = encodeRdb(data);
  assert.deepEqual(decodeRdb(encoded),data);
  assert.deepEqual(encodeRdb(decodeRdb(encoded)), encoded);
});

test("TDD RED: binary byte fields decoded from Node Socket Buffer are plain Uint8Array", () => {
  const original = new Map([["binary", Uint8Array.from([0,255,128])]]);
  const decoded = decodeRdb(Buffer.from(encodeRdb(original)));
  assert.deepEqual(decoded, original);
  assert.equal(Object.getPrototypeOf(decoded.get("binary")), Uint8Array.prototype,
    "RDB byte fields must be plain Uint8Array, not Buffer");
});
