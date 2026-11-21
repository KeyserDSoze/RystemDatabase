# @rystemdb/client (TypeScript / Node.js)

[TypeScript SDK Issue #28](https://github.com/KeyserDSoze/RystemDatabase/issues/28).

**Implemented:** TypeScript `src/wire.ts` RSWP v1 frame `encode/decode` with exact 64-bit `bigint` correlation IDs, bounded binary bodies, version/status/length checks and Rust PING golden vector tests. Test with Node.js 22+:

```sh
npm test
```

**Not implemented:** an operational TCP/TLS database client, session pooling, cancellation, typed CRUD, cluster routing, browser support or published npm SDK. This SDK is independent of the `npm/cli` launcher package. Do not advertise it as a working remote client before server #25 and end-to-end tests.
