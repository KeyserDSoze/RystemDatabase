/** Experimental cross-language RDB binary document codec; RED TDD stub. */
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
export function encodeRdb(_value: RdbValue): Uint8Array {
  throw new Error("typed RDB v1 encoder not implemented");
}
export function decodeRdb(_bytes: Uint8Array): RdbValue {
  throw new Error("typed RDB v1 decoder not implemented");
}
