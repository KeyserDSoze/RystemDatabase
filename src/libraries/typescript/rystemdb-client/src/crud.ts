/** Bounded RSWP v1 CRUD key prefix. Independent Rust and .NET golden tests.
 * Binary layout: BE u16 ASCII collection length, collection, BE u16 UTF-8 ID
 * length, ID; any remaining bytes are opaque typed RDB document bytes for PUT.
 * This is a codec, NOT an authenticated or working remote database client.
 */
export interface CrudKey {
  collection: string;
  id: string;
  document: Uint8Array;
}
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const MAX_BODY = 16 * 1024 * 1024;

function validCollection(s: string): boolean {
  return /^[a-zA-Z0-9_-]{1,128}$/.test(s);
}
function validId(s: string, collectionLength: number): boolean {
  if (!s || /\p{Cc}/u.test(s)) return false;
  // JavaScript strings may contain unpaired UTF-16 surrogate code units.
  // The default TextEncoder replaces those silently; reject instead.
  for (const cp of s) {
    const code = cp.codePointAt(0)!;
    if (code >= 0xd800 && code <= 0xdfff) return false;
  }
  return collectionLength + 1 + encoder.encode(s).length <= 1024;
}
export function encodeCrudKey(collection: string, id: string): Uint8Array {
  if (!validCollection(collection)) throw new Error("invalid collection");
  if (!validId(id, collection.length)) throw new Error("invalid ID");
  const collectionBytes = encoder.encode(collection);
  const idBytes = encoder.encode(id);
  const out = new Uint8Array(4 + collectionBytes.length + idBytes.length);
  const view = new DataView(out.buffer);
  view.setUint16(0, collectionBytes.length, false);
  out.set(collectionBytes, 2);
  const idOffset = 2 + collectionBytes.length;
  view.setUint16(idOffset, idBytes.length, false);
  out.set(idBytes, idOffset + 2);
  return out;
}
export function decodeCrudKey(bytes: Uint8Array): CrudKey {
  if (!(bytes instanceof Uint8Array)) throw new Error("invalid CRUD bytes");
  if (bytes.length > MAX_BODY) throw new Error("CRUD body too large");
  if (bytes.length < 2) throw new Error("truncated collection length");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const collectionLength = view.getUint16(0, false);
  if (collectionLength === 0 || collectionLength > 128) throw new Error("invalid collection length");
  const idOffset = 2 + collectionLength;
  if (bytes.length < idOffset + 2) throw new Error("truncated collection or ID length");
  let collection: string;
  try {
    collection = decoder.decode(bytes.subarray(2, idOffset));
  } catch {
    throw new Error("invalid collection UTF-8");
  }
  if (!validCollection(collection)) throw new Error("invalid collection");
  const idLength = view.getUint16(idOffset, false);
  if (idLength === 0 || collectionLength + 1 + idLength > 1024) throw new Error("invalid ID length");
  const end = idOffset + 2 + idLength;
  if (bytes.length < end) throw new Error("truncated ID bytes");
  let id: string;
  try {
    id = decoder.decode(bytes.subarray(idOffset + 2, end));
  } catch {
    throw new Error("invalid ID UTF-8");
  }
  if (!validId(id, collectionLength)) throw new Error("invalid ID");
  return { collection, id, document: bytes.slice(end) };
}
