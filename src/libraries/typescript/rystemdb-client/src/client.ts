/**
 * Experimental localhost-only RSWP v1 TCP client; optional local token,
 * plus explicit certificate-verified TLS1.3. NEVER supports remote bindings.
 * Concurrent requests are correlated by u64 ID on one persistent socket.
 * Incomplete, invalid or timed-out connections fail closed; writes are not
 * automatically retried because the server-side outcome may be uncertain.
 */
import { Socket } from "node:net";
import { connect as connectTlsSocket, type TLSSocket } from "node:tls";
import { encode, decode, decodeHeader, HEADER_LEN, MAX_PAYLOAD_BYTES, type Frame,
  type Operation } from "./wire.ts";
import { encodeCrudKey } from "./crud.ts";
import { encodeRdb, decodeRdb, type RdbValue } from "./rdb.ts";

const TIMEOUT_MS = 3000;
const U64_MAX = (1n << 64n) - 1n;
const MAX_PENDING = 120;
// #76: limit the unflushed native TCP/TLS writable queue to one maximum RSWP
// frame; the separate 120 in-flight request cap does NOT bound byte memory.
const MAX_OUTBOUND_BUFFER_BYTES = MAX_PAYLOAD_BYTES + HEADER_LEN;

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
  // #74: one fixed 24-byte header and one bounded, validated in-flight
  // frame; never re-concatenate a growing body on every TCP/TLS fragment.
  private readonly receiveHeader = Buffer.alloc(HEADER_LEN);
  private receivedHeaderBytes = 0;
  private receiveFrame: Buffer | null = null;
  private receivedFrameBytes = 0;
  private nextId = 1n;
  private closed = false;
  private writeBackpressured = false;

  private constructor(socket: Socket) {
    this.socket = socket;
    socket.setNoDelay(true);
    socket.setTimeout(TIMEOUT_MS, () => {
      socket.destroy(new Error("Rystem RPC socket inactivity timeout"));
    });
    socket.on("data", (chunk: Buffer) => this.onData(chunk));
    // Node write(false) means "stop queuing"; a subsequent drain is the
    // ONLY signal that the writable stream can accept another frame.
    socket.on("drain", () => { this.writeBackpressured = false; });
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

  /**
   * Real TLS1.3-only client to an explicit numeric loopback target.
   * Node validates the server chain against the caller-supplied root and the
   * DNS SAN against serverName. Only after 'secureConnect' with authorized=true
   * do we send the RSWP bearer token. No rejectUnauthorized=false, no custom
   * certificate bypass, no plaintext fallback or external network access.
   *
   * Optional client cert + key opts into experimental loopback mTLS (#68).
   * Native node:tls presents the client certificate and proves key ownership;
   * the Rust server separately enforces the verified principal/token ACL.
   * Both PEM credentials must be explicitly supplied together and bounded.
   * Production remote mTLS/revocation/credential lifecycle remain unavailable.
   */
  static async connectTls(options: {
    host: string; port: number; serverName: string; ca: string | Uint8Array; token: Uint8Array;
    cert?: string | Uint8Array; key?: string | Uint8Array
  }): Promise<RystemLocalClient> {
    if (!options || (options.host !== "127.0.0.1" && options.host !== "::1")) {
      throw new Error("Rystem TLS client requires a numeric loopback host");
    }
    if (!Number.isInteger(options.port) || options.port < 1 || options.port > 65535) {
      throw new Error("Invalid Rystem TLS client port");
    }
    if (typeof options.serverName !== "string" || !options.serverName.trim()) {
      throw new Error("Expected a TLS server DNS name for certificate SAN verification");
    }
    if (!(options.token instanceof Uint8Array) || options.token.length !== 32) {
      throw new Error("Rystem TLS client requires exactly 32 raw token bytes");
    }
    if (!(typeof options.ca === "string" ||
          options.ca instanceof Uint8Array) || options.ca.length === 0 ||
        options.ca.length > 65536) {
      throw new Error("Rystem TLS client requires an explicit bounded root CA PEM");
    }
    // #68: On the verified TLS entrypoint, never silently discard a
    // supplied client identity or attempt a token-only mTLS handshake.
    // No path or PEM key contents are passed to spawned processes or logs.
    const haveCert = options.cert !== undefined;
    const haveKey = options.key !== undefined;
    if (haveCert !== haveKey) {
      throw new Error("Client certificate and private key PEM must be provided together");
    }
    const boundedPem = (value: string | Uint8Array, label: string): Buffer => {
      if (!(typeof value === "string" || value instanceof Uint8Array)) {
        throw new Error("Invalid client " + label + " PEM");
      }
      const raw = Buffer.from(value);
      if (!raw.length || raw.length > 65536) {
        throw new Error("Invalid or oversized client " + label + " PEM");
      }
      const text = raw.toString("utf8");
      const expected = label === "certificate" ?
        /-----BEGIN CERTIFICATE-----\r?\n[\s\S]+-----END CERTIFICATE-----/ :
        /-----BEGIN (?:RSA |EC )?PRIVATE KEY-----\r?\n[\s\S]+-----END (?:RSA |EC )?PRIVATE KEY-----/;
      if (!expected.test(text)) {
        throw new Error("Malformed client " + label + " PEM");
      }
      return raw;
    };
    const clientCert = haveCert ? boundedPem(options.cert!, "certificate") : undefined;
    const clientKey = haveKey ? boundedPem(options.key!, "private key") : undefined;
    const root = typeof options.ca === "string" ? options.ca : Buffer.from(options.ca);
    const token = Uint8Array.from(options.token);
    const socket: TLSSocket = connectTlsSocket({
      host: options.host,
      port: options.port,
      servername: options.serverName,
      ca: root,
      ...(clientCert ? {cert: clientCert, key: clientKey} : {}),
      minVersion: "TLSv1.3",
      maxVersion: "TLSv1.3",
      rejectUnauthorized: true,
    });
    const client = new RystemLocalClient(socket);
    try {
      await new Promise<void>((resolve, reject) => {
        const cleanup = () => {
          socket.off("secureConnect", secure);
          socket.off("error", fail);
          socket.off("close", closed);
        };
        const secure = () => {
          cleanup();
          if (!socket.authorized || socket.getProtocol() !== "TLSv1.3") {
            reject(new Error("Rystem TLS 1.3 certificate verification failed"));
          } else {
            resolve();
          }
        };
        const fail = (error: Error) => { cleanup(); reject(error); };
        const closed = () => { cleanup(); reject(new Error("TLS socket closed before authentication")); };
        socket.once("secureConnect", secure);
        socket.once("error", fail);
        socket.once("close", closed);
      });
      const hello = await client.request(1, Uint8Array.from([1, 0, 0, 32, ...token]));
      if (hello.status !== 0) throw new RystemRpcError(hello.status);
      if (hello.payload.length !== 2 || hello.payload[0] !== 1 || hello.payload[1] !== 0) {
        throw new Error("Rystem TLS server returned invalid RSWP HELLO");
      }
      return client;
    } catch (error) {
      socket.destroy();
      throw error;
    } finally {
      token.fill(0);
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
    const id = this.nextId;
    const bytes = encode({ kind: 1, operation, requestId: id, status: 0, payload });
    const buffered = this.socket.writableLength;
    // Check BEFORE handing ownership to node:net/node:tls. Unsent requests
    // may be retried explicitly by the caller, whereas a write already
    // accepted by the socket has an UNCERTAIN outcome on failure.
    if (this.writeBackpressured || this.socket.writableNeedDrain ||
        !Number.isSafeInteger(buffered) || buffered < 0 ||
        buffered > MAX_OUTBOUND_BUFFER_BYTES - bytes.length) {
      return Promise.reject(new Error(
        "Rystem RPC outgoing write backpressure: request not sent"
      ));
    }
    this.nextId += 1n;
    return new Promise<Frame>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.socket.destroy(new Error("Rystem RPC request timeout"));
      }, TIMEOUT_MS);
      timer.unref();
      this.pending.set(id, { operation, resolve, reject, timer });
      try {
        // A false return does NOT mean this request failed: it was queued.
        // Wait for "drain" before accepting any subsequent outgoing frame.
        if (!this.socket.write(bytes)) {
          this.writeBackpressured = true;
        }
      } catch (error) {
        // A synchronous transport failure must not leave an orphaned waiter
        // or timer. Tear down the connection; other accepted writes may
        // already be in flight and must not be silently retried.
        const failure = error instanceof Error ? error :
          new Error("Rystem RPC synchronous socket write failure");
        this.pending.delete(id);
        clearTimeout(timer);
        reject(failure);
        this.socket.destroy(failure);
      }
    });
  }

  private onData(chunk: Buffer): void {
    try {
      let offset = 0;
      while (offset < chunk.length) {
        if (this.receiveFrame === null) {
          // Header can be split across arbitrary TCP/TLS chunks. Copy each
          // header byte once, then reject malformed/unexpected peers BEFORE
          // allocating the body buffer, even for claimed 16 MiB responses.
          const count = Math.min(HEADER_LEN - this.receivedHeaderBytes, chunk.length - offset);
          chunk.copy(this.receiveHeader, this.receivedHeaderBytes, offset, offset + count);
          this.receivedHeaderBytes += count;
          offset += count;
          if (this.receivedHeaderBytes < HEADER_LEN) break;
          const header = decodeHeader(this.receiveHeader);
          const waiter = this.pending.get(header.requestId);
          if (!waiter || header.kind !== 2 || header.operation !== waiter.operation) {
            throw new Error("Unexpected or mismatched RSWP response correlation");
          }
          const frame = Buffer.allocUnsafe(HEADER_LEN + header.payloadLength);
          this.receiveHeader.copy(frame, 0);
          this.receiveFrame = frame;
          this.receivedFrameBytes = HEADER_LEN;
          this.receivedHeaderBytes = 0;
        }

        const frame = this.receiveFrame;
        if (frame === null) throw new Error("Missing RSWP frame");
        // Each body byte is copied at most once into the pre-sized frame.
        const count = Math.min(frame.length - this.receivedFrameBytes, chunk.length - offset);
        if (count > 0) {
          chunk.copy(frame, this.receivedFrameBytes, offset, offset + count);
          this.receivedFrameBytes += count;
          offset += count;
        }
        if (this.receivedFrameBytes === frame.length) {
          const response = decode(frame);
          // Release the buffer before calling user-facing promise callbacks.
          // Any trailing bytes in this same chunk are a new independent frame.
          this.receiveFrame = null;
          this.receivedFrameBytes = 0;
          const waiter = this.pending.get(response.requestId);
          if (!waiter || response.kind !== 2 || response.operation !== waiter.operation) {
            throw new Error("Unexpected or mismatched RSWP response correlation");
          }
          this.pending.delete(response.requestId);
          clearTimeout(waiter.timer);
          waiter.resolve(response);
        }
      }
    } catch (error) {
      this.socket.destroy(error instanceof Error ? error : new Error(String(error)));
    }
  }

  private failAll(error: Error): void {
    if (this.closed && this.pending.size === 0) return;
    this.closed = true;
    this.writeBackpressured = false;
    this.receivedHeaderBytes = 0;
    this.receivedFrameBytes = 0;
    this.receiveFrame = null;
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
