/**
 * RDB\0 binary typed value codec, format version 1. Cross-language equivalent
 * of rystemdb-core::Value, without JSON or lossy Number coercion of integers.
 *
 * Representation: bigint=i64, number=f64, Uint8Array=bytes,
 * Map<string,RdbValue>=object (missing keys ≠ present null), arrays, strings,
 * booleans, null, and RdbReference for native typed references.
 * This codec does not establish an authenticated network connection.
 */
export class RdbReference {
  readonly collection: string;
  readonly id: string;
  constructor(collection: string, id: string) {
    this.collection = collection;
    this.id = id;
  }
}

export type RdbValue = null | boolean | bigint | number | string | Uint8Array |
  RdbReference | RdbValue[] | Map<string, RdbValue>;

const MAX_BYTES = 64 * 1024 * 1024;
const MAX_DEPTH = 64;
const INT64_MIN = -(1n << 63n);
const INT64_MAX = (1n << 63n) - 1n;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: true });

function utf8(value: string): Uint8Array {
  if (typeof value !== "string") throw new Error("expected string");
  for (const cp of value) {
    const n = cp.codePointAt(0)!;
    if (n >= 0xd800 && n <= 0xdfff) {
      throw new Error("invalid Unicode surrogate");
    }
  }
  return textEncoder.encode(value);
}

class Writer {
  private buf = new Uint8Array(256);
  private length = 0;

  private ensure(additional: number): void {
    const size = this.length + additional;
    if (!Number.isSafeInteger(size) || size > MAX_BYTES) throw new Error("RDB document too large");
    if (size <= this.buf.length) return;
    let capacity = this.buf.length;
    while (capacity < size) capacity = Math.min(MAX_BYTES, capacity * 2);
    const next = new Uint8Array(capacity);
    next.set(this.buf.subarray(0, this.length));
    this.buf = next;
  }
  byte(value: number): void {
    this.ensure(1);
    this.buf[this.length++] = value;
  }
  bytes(value: Uint8Array): void {
    this.ensure(value.length);
    this.buf.set(value, this.length);
    this.length += value.length;
  }
  u32(value: number): void {
    if (!Number.isSafeInteger(value) || value < 0 || value > 0xffff_ffff) {
      throw new Error("RDB length out of range");
    }
    this.ensure(4);
    new DataView(this.buf.buffer).setUint32(this.length, value, false);
    this.length += 4;
  }
  i64(value: bigint): void {
    if (value < INT64_MIN || value > INT64_MAX) throw new Error("i64 integer out of range");
    this.ensure(8);
    new DataView(this.buf.buffer).setBigInt64(this.length, value, false);
    this.length += 8;
  }
  f64(value: number): void {
    if (!Number.isFinite(value)) throw new Error("non-finite floating-point value");
    this.ensure(8);
    new DataView(this.buf.buffer).setFloat64(this.length, value, false);
    this.length += 8;
  }
  text(value: string): void {
    const raw = utf8(value);
    this.u32(raw.length);
    this.bytes(raw);
  }
  result(): Uint8Array {
    return this.buf.slice(0, this.length);
  }
}
function compareBytes(a: Uint8Array, b: Uint8Array): number {
  for (let i=0; i<Math.min(a.length,b.length); i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}
function encodeValue(w: Writer, value: RdbValue, depth: number): void {
  if (depth > MAX_DEPTH) throw new Error("RDB nesting too deep");
  if (value === null) { w.byte(0); return; }
  if (typeof value === "boolean") { w.byte(value ? 2 : 1); return; }
  if (typeof value === "bigint") { w.byte(3); w.i64(value); return; }
  if (typeof value === "number") { w.byte(4); w.f64(value); return; }
  if (typeof value === "string") { w.byte(5); w.text(value); return; }
  if (value instanceof Uint8Array) { w.byte(6); w.u32(value.length); w.bytes(value); return; }
  if (Array.isArray(value)) {
    w.byte(7); w.u32(value.length);
    for(const item of value) encodeValue(w,item,depth+1);
    return;
  }
  if (value instanceof Map) {
    const fields = [...value.entries()].map(([key,v]) => ({ key: utf8(key), value: v }));
    fields.sort((a,b) => compareBytes(a.key,b.key)); // matches Rust BTreeMap UTF-8 ordering
    w.byte(8); w.u32(fields.length);
    for (const field of fields) {
      w.u32(field.key.length); w.bytes(field.key);
      encodeValue(w,field.value,depth+1);
    }
    return;
  }
  if (value instanceof RdbReference) {
    if (!value.collection || !value.id) throw new Error("invalid RDB reference");
    w.byte(9); w.text(value.collection); w.text(value.id);
    return;
  }
  throw new Error("invalid RDB value type; use Map for documents and bigint for i64");
}

export function encodeRdb(value: RdbValue): Uint8Array {
  const w = new Writer();
  w.bytes(Uint8Array.from([82,68,66,0,1])); // magic + version
  encodeValue(w,value,0);
  return w.result();
}

class Reader {
  private pos = 0;
  private readonly view: DataView;
  constructor(private readonly bytes: Uint8Array) {
    this.view = new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength);
  }
  get remaining(): number { return this.bytes.length - this.pos; }
  byte(): number { return this.take(1)[0]; }
  take(length: number): Uint8Array {
    if (!Number.isSafeInteger(length) || length < 0 || length > this.remaining) {
      throw new Error("truncated RDB document");
    }
    const chunk = this.bytes.subarray(this.pos,this.pos+length);
    this.pos += length;
    return chunk;
  }
  u32(): number {
    if (this.remaining < 4) throw new Error("truncated RDB length");
    const result = this.view.getUint32(this.pos,false); this.pos+=4;
    return result;
  }
  i64(): bigint {
    if (this.remaining < 8) throw new Error("truncated RDB i64");
    const result = this.view.getBigInt64(this.pos,false); this.pos+=8;
    return result;
  }
  f64(): number {
    if (this.remaining < 8) throw new Error("truncated RDB f64");
    const result = this.view.getFloat64(this.pos,false); this.pos+=8;
    if (!Number.isFinite(result)) throw new Error("non-finite floating-point value");
    return result;
  }
  text(): string {
    const raw = this.take(this.u32());
    try { return textDecoder.decode(raw); }
    catch { throw new Error("invalid UTF-8 in RDB string"); }
  }
}
function decodeValue(r: Reader, depth: number): RdbValue {
  if (depth > MAX_DEPTH) throw new Error("RDB nesting too deep");
  const tag = r.byte();
  switch(tag) {
    case 0: return null;
    case 1: return false;
    case 2: return true;
    case 3: return r.i64();
    case 4: return r.f64();
    case 5: return r.text();
    case 6: return r.take(r.u32()).slice();
    case 7: {
      const count = r.u32();
      if (count > r.remaining) throw new Error("truncated RDB array");
      const values: RdbValue[] = [];
      for(let i=0;i<count;i++) values.push(decodeValue(r,depth+1));
      return values;
    }
    case 8: {
      const count = r.u32();
      if (count > Math.floor(r.remaining/5)) throw new Error("truncated RDB object");
      const values = new Map<string,RdbValue>();
      for(let i=0;i<count;i++) {
        const key = r.text();
        const value = decodeValue(r,depth+1);
        if (values.has(key)) throw new Error("duplicate RDB field");
        values.set(key,value);
      }
      return values;
    }
    case 9: {
      const collection = r.text(); const id = r.text();
      if (!collection || !id) throw new Error("invalid RDB reference");
      return new RdbReference(collection,id);
    }
    default: throw new Error("invalid RDB type tag");
  }
}

export function decodeRdb(bytes: Uint8Array): RdbValue {
  if (!(bytes instanceof Uint8Array)) throw new Error("invalid RDB bytes");
  if (bytes.length > MAX_BYTES) throw new Error("RDB document too large");
  const r = new Reader(bytes);
  if (r.remaining < 5) throw new Error("truncated RDB header");
  const header = r.take(5);
  if (header[0]!==82 || header[1]!==68 || header[2]!==66 || header[3]!==0) {
    throw new Error("invalid RDB magic");
  }
  if (header[4]!==1) throw new Error("unsupported RDB version");
  const result = decodeValue(r,0);
  if (r.remaining !== 0) throw new Error("trailing RDB bytes");
  return result;
}
