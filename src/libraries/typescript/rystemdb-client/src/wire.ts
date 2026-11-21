/**
 * Rystem Wire Protocol v1 (RSWP): strict, bounded binary frame codec.
 * This is only the transport framing layer, NOT an authenticated SDK connection.
 * The native Rust protocol crate defines the reference octets.
 */
export const HEADER_LEN = 24;
export const MAX_PAYLOAD_BYTES = 16 * 1024 * 1024;
const U64_MAX = (1n << 64n) - 1n;

export type Kind = 1 | 2;
export type Operation = 1 | 2 | 16 | 17 | 18;

export interface Frame {
  readonly kind: Kind;
  readonly operation: Operation;
  readonly requestId: bigint;
  /** 0=success, 1=not found, 2=invalid request, 3=busy, etc. */
  readonly status: number;
  readonly payload: Uint8Array;
}

const allowedOperations = new Set<number>([1,2,16,17,18]);
function validateKindAndStatus(kind: number, status: number): void {
  if (kind !== 1 && kind !== 2) throw new Error("invalid frame kind");
  if (!Number.isInteger(status) || status < 0 || status > 7 || (kind === 1 && status !== 0)) {
    throw new Error("invalid request/response status");
  }
}

export function encode(frame: Frame): Uint8Array {
  validateKindAndStatus(frame.kind, frame.status);
  if (!allowedOperations.has(frame.operation)) throw new Error("invalid operation");
  if (typeof frame.requestId !== "bigint" || frame.requestId < 0n || frame.requestId > U64_MAX) {
    throw new Error("request ID must be an unsigned 64-bit bigint");
  }
  if (!(frame.payload instanceof Uint8Array)) throw new Error("payload must be bytes");
  if (frame.payload.length > MAX_PAYLOAD_BYTES) throw new Error("payload too large");
  const bytes = new Uint8Array(HEADER_LEN + frame.payload.byteLength);
  const view = new DataView(bytes.buffer);
  bytes.set([0x52, 0x53, 0x57, 0x50], 0);
  bytes[4] = 1; bytes[5] = 0;
  bytes[6] = frame.kind; bytes[7] = frame.operation;
  view.setUint16(8, frame.status, false);
  view.setUint16(10, HEADER_LEN, false);
  view.setBigUint64(12, frame.requestId, false);
  view.setUint32(20, frame.payload.length, false);
  bytes.set(frame.payload, HEADER_LEN);
  return bytes;
}

export function decode(input: Uint8Array): Frame {
  if (input.length < HEADER_LEN) throw new Error("incomplete frame header");
  const view = new DataView(input.buffer, input.byteOffset, input.byteLength);
  if (input[0] !== 0x52 || input[1] !== 0x53 || input[2] !== 0x57 || input[3] !== 0x50) {
    throw new Error("invalid frame magic");
  }
  if (input[4] !== 1 || input[5] !== 0) throw new Error("unsupported protocol version");
  const kind = input[6];
  const operation = input[7];
  const status = view.getUint16(8, false);
  validateKindAndStatus(kind, status);
  if (!allowedOperations.has(operation)) throw new Error("invalid operation");
  if (view.getUint16(10, false) !== HEADER_LEN) throw new Error("invalid header length");
  const length = view.getUint32(20, false);
  if (length > MAX_PAYLOAD_BYTES) throw new Error("frame too large");
  const expected = HEADER_LEN + length;
  if (input.length < expected) throw new Error("incomplete frame payload");
  if (input.length > expected) throw new Error("trailing bytes after frame");
  return {
    kind: kind as Kind,
    operation: operation as Operation,
    status,
    requestId: view.getBigUint64(12, false),
    payload: input.slice(HEADER_LEN),
  };
}
