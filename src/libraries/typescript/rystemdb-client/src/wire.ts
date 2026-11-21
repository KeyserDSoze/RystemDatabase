/**
 * RSWP v1 binary framing foundation. Remote network client is a separate feature (#28).
 * This file is deliberately RED until the cross-language contract is implemented.
 */
export const HEADER_LEN = 24;
export const MAX_PAYLOAD_BYTES = 16 * 1024 * 1024;

export type Kind = 1 | 2;
export type Operation = 1 | 2 | 16 | 17 | 18;
export interface Frame {
  readonly kind: Kind;
  readonly operation: Operation;
  readonly requestId: bigint;
  readonly status: number;
  readonly payload: Uint8Array;
}

export function encode(_frame: Frame): Uint8Array {
  throw new Error("RSWP codec not implemented");
}

export function decode(_input: Uint8Array): Frame {
  throw new Error("RSWP codec not implemented");
}
