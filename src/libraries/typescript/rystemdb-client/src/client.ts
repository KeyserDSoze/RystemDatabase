/**
 * Experimental localhost-only RSWP v1 TCP client; optional local token,
 * but still NO TLS or remote authenticated transport.
 * Concurrent requests are correlated by u64 ID on one persistent socket.
 * Incomplete, invalid or timed-out connections fail closed; writes are not
 * automatically retried because the server-side outcome may be uncertain.
 */
import { Socket } from "node:net";
import { encode, decode, HEADER_LEN, MAX_PAYLOAD_BYTES, type Frame,
  type Operation } from "./wire.ts";
import { encodeCrudKey } from "./crud.ts";
import { encodeRdb, decodeRdb, type RdbValue } from "./rdb.ts";

const TIMEOUT_MS = 3000;
const U64_MAX = (1n << 64n) - 1n;
const MAX_PENDING = 120;

export class RystemRpcError extends Error {
  readonly status: number;
  constructor(status: number) {
    super("Rystem Database RPC rejected the operation (status " + status + ")");
    this.name = "RystemRpcError";
    this.status = status;
  }
}

interface PendingCall {
  operation: Operation;
  resolve: (frame: Frame) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class RystemLocalClient {
  private readonly socket: Socket;
  private readonly pending = new Map<bigint, PendingCall>();
  private receiveBuffer = Buffer.alloc(0);
  private nextId = 1n;
  private closed = false;

  private constructor(socket: Socket) {
    this.socket = socket;
    socket.setNoDelay(true);
    socket.setTimeout(TIMEOUT_MS, () => {
      socket.destroy(new Error("Rystem RPC socket inactivity timeout"));
    });
    socket.on("data", (chunk: Buffer) => this.onData(chunk));
    socket.on("error", (error: Error) => this.failAll(error));
    socket.on("close", () => this.failAll(new Error("Rystem RPC connection closed")));
  }

  static async connect(host: string, port: number, token?: Uint8Array): Promise<RystemLocalClient> {
    if (host !== "127.0.0.1" && host !== "::1") {
      throw new Error("Only numeric loopback addresses are allowed without TLS/auth");
    }
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      throw new Error("Invalid Rystem RPC port");
    }
    if (token !== undefined &&
        (!(token instanceof Uint8Array) || token.length !== 32)) {
      throw new Error("Rystem local token must be exactly 32 raw bytes");
    }
    const socket = new Socket();
    const client = new RystemLocalClient(socket);
    try {
      await new Promise<void>((resolve, reject) => {
        const onError = (err: Error) => {
          socket.off("connect", onConnect);
          reject(err);
        };
        const onConnect = () => {
          socket.off("error", onError);
          resolve();
        };
        socket.once("error", onError);
        socket.once("connect", onConnect);
        socket.connect(port, host);
      });
      const handshake = token === undefined ? Uint8Array.from([1, 0]) :
        Uint8Array.from([1, 0, 0, 32, ...token]);
      const hello = await client.request(1, handshake);
      if (hello.status !== 0) throw new RystemRpcError(hello.status);
      if (hello.payload.length !== 2 ||
          hello.payload[0] !== 1 || hello.payload[1] !== 0) {
        throw new Error("Rystem server returned malformed RSWP v1 HELLO response");
      }
      return client;
    } catch (error) {
      socket.destroy();
      throw error;
    }
  }

  async ping(): Promise<void> {
    const frame = await this.request(2, new Uint8Array());
    this.expectStatus(frame, 0);
    if (frame.payload.length) throw new Error("Nonempty PING response");
  }

  async put(collection: string, id: string, document: RdbValue): Promise<void> {
    if (!(document instanceof Map)) throw new Error("RPC PUT requires an RDB object Map");
    const key = encodeCrudKey(collection, id);
    const binary = encodeRdb(document);
    if (key.length + binary.length > MAX_PAYLOAD_BYTES) {
      throw new Error("Rystem RPC PUT payload too large");
    }
    const payload = new Uint8Array(key.length + binary.length);
    payload.set(key);
    payload.set(binary, key.length);
    const frame = await this.request(16, payload);
    this.expectStatus(frame, 0);
    if (frame.payload.length) throw new Error("Nonempty PUT response");
  }

  async get(collection: string, id: string): Promise<RdbValue | null> {
    const frame = await this.request(17, encodeCrudKey(collection, id));
    if (frame.status === 1 && frame.payload.length === 0) return null;
    this.expectStatus(frame, 0);
    const value = decodeRdb(frame.payload);
    if (!(value instanceof Map)) throw new Error("Expected native typed RDB object");
    return value;
  }

  async delete(collection: string, id: string): Promise<boolean> {
    const frame = await this.request(18, encodeCrudKey(collection, id));
    if (frame.payload.length) throw new Error("Nonempty DELETE response");
    if (frame.status === 1) return false;
    this.expectStatus(frame, 0);
    return true;
  }

  private expectStatus(response: Frame, wanted: number): void {
    if (response.status !== wanted) throw new RystemRpcError(response.status);
  }

  private request(operation: Operation, payload: Uint8Array): Promise<Frame> {
    if (this.closed || this.socket.destroyed) {
      return Promise.reject(new Error("Rystem TCP connection is closed"));
    }
    if (this.pending.size >= MAX_PENDING) {
      return Promise.reject(new Error("Rystem RPC request queue is full"));
    }
    if (this.nextId > U64_MAX) {
      return Promise.reject(new Error("Rystem RPC correlation IDs exhausted"));
    }
    const id = this.nextId++;
    const bytes = encode({ kind: 1, operation, requestId: id, status: 0, payload });
    return new Promise<Frame>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.socket.destroy(new Error("Rystem RPC request timeout"));
      }, TIMEOUT_MS);
      timer.unref();
      this.pending.set(id, { operation, resolve, reject, timer });
      this.socket.write(bytes);
    });
  }

  private onData(chunk: Buffer): void {
    try {
      const total = this.receiveBuffer.length + chunk.length;
      if (total > MAX_PAYLOAD_BYTES + HEADER_LEN) {
        throw new Error("Buffered Rystem RPC body too large");
      }
      this.receiveBuffer = Buffer.concat([this.receiveBuffer, chunk], total);
      while (this.receiveBuffer.length >= HEADER_LEN) {
        const length = this.receiveBuffer.readUInt32BE(20);
        if (length > MAX_PAYLOAD_BYTES) throw new Error("Rystem RPC payload too large");
        const required = HEADER_LEN + length;
        if (this.receiveBuffer.length < required) break;
        const response = decode(this.receiveBuffer.subarray(0, required));
        this.receiveBuffer = this.receiveBuffer.subarray(required);
        const waiter = this.pending.get(response.requestId);
        if (!waiter || response.kind !== 2 || response.operation !== waiter.operation) {
          throw new Error("Unexpected or mismatched RSWP response correlation");
        }
        this.pending.delete(response.requestId);
        clearTimeout(waiter.timer);
        waiter.resolve(response);
      }
    } catch (error) {
      this.socket.destroy(error instanceof Error ? error : new Error(String(error)));
    }
  }

  private failAll(error: Error): void {
    if (this.closed && this.pending.size === 0) return;
    this.closed = true;
    this.receiveBuffer = Buffer.alloc(0);
    for (const waiter of this.pending.values()) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
    this.pending.clear();
  }

  async close(): Promise<void> {
    this.socket.destroy();
    this.failAll(new Error("Rystem RPC client closed"));
  }
}
