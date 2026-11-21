# @rystemdb/client (TypeScript / Node.js)

[TypeScript SDK Issue #28](https://github.com/KeyserDSoze/RystemDatabase/issues/28).

**Implemented:** TypeScript `src/wire.ts` RSWP v1 frame `encode/decode` with exact 64-bit `bigint` correlation IDs, bounded binary bodies, version/status/length checks and Rust PING golden vector tests. Test with Node.js 22+:

```sh
npm test
```

**Not implemented:** an operational TCP/TLS database client, session pooling, cancellation, typed CRUD, cluster routing, browser support or published npm SDK. This SDK is independent of the `npm/cli` launcher package. Do not advertise it as a working remote client before server #25 and end-to-end tests.

## Binary CRUD key codec (source only)

`encodeCrudKey/decodeCrudKey` in `src/crud.ts` use strict UTF-8 and exact big-endian lengths, reject malformed/truncated data and isolated UTF-16 surrogates, and preserve opaque RDB PUT bytes. `npm test` exercises the same golden vectors as Rust and C#. **No typed RDB decoder or TCP/TLS connection yet** (#43 / #28).
