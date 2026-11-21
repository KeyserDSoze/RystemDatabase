import test from "node:test";
import assert from "node:assert/strict";
import { encodeCrudKey, decodeCrudKey } from "../src/crud.ts";

const ascii = Uint8Array.from([0,5,117,115,101,114,115,0,3,117,52,50]);
const unicode = Uint8Array.from([0,6,111,114,100,101,114,115,0,4,240,159,166,138]);
const doc = Uint8Array.from([
  82,68,66,0,1,8,0,0,0,1,0,0,0,1,110,3,0,0,0,0,0,0,0,42,
]);

test("TDD RED: TS GET key uses same golden bytes as Rust and C#", () => {
  assert.deepEqual(encodeCrudKey("users","u42"), ascii);
  assert.deepEqual(decodeCrudKey(ascii), {collection:"users",id:"u42",document:new Uint8Array()});
});
test("TDD RED: UTF-8 ID plus binary PUT bytes are not converted to JSON", () => {
  assert.deepEqual(encodeCrudKey("orders","🦊"),unicode);
  const put = Uint8Array.from([...unicode,...doc]);
  assert.deepEqual(decodeCrudKey(put),{collection:"orders",id:"🦊",document:doc});
});
test("TDD RED: collection/ID restrictions match Rust", () => {
  for(const collection of ["","bad/name","a b","é","x".repeat(129)]) {
    assert.throws(()=>encodeCrudKey(collection,"id"),/collection/i);
  }
  for(const id of ["","\n","\u0085","x".repeat(1024)]) {
    assert.throws(()=>encodeCrudKey("users",id),/id/i);
  }
});
test("TDD RED: truncated, invalid UTF-8 and malformed lengths fail closed", () => {
  for(let i=0;i<ascii.length;i++){
    assert.throws(()=>decodeCrudKey(ascii.subarray(0,i)),/truncat|incomplete|length/i);
  }
  const invalid = Uint8Array.from(ascii);
  invalid[10]=255;
  assert.throws(()=>decodeCrudKey(invalid),/utf|id/i);
  assert.throws(()=>decodeCrudKey(Uint8Array.from([0,1,47,0,1,120])),/collection/i);
});

test("TDD RED: leading U+FEFF is data in a UTF-8 document ID, not a file BOM", () => {
  const id = "\uFEFFid";
  const encoded = encodeCrudKey("docs", id);
  assert.deepEqual(decodeCrudKey(encoded), {
    collection: "docs", id, document: new Uint8Array()
  });
  assert.deepEqual(encodeCrudKey("docs", decodeCrudKey(encoded).id), encoded);
});
