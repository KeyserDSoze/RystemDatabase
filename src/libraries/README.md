# Client libraries for Rystem Database

This folder has four libraries:
- `dotnet/RystemDatabase.Client`: .NET (C#) RPC client, [#27](https://github.com/KeyserDSoze/RystemDatabase/issues/27)
- `rust/rystemdb-client`: Rust typed RPC client, [#26](https://github.com/KeyserDSoze/RystemDatabase/issues/26)
- `typescript/rystemdb-client`: TypeScript/Node.js RPC client, [#28](https://github.com/KeyserDSoze/RystemDatabase/issues/28)
- `dotnet/RystemDatabase.EntityFrameworkCore`: EF Core provider, [#29](https://github.com/KeyserDSoze/RystemDatabase/issues/29)

Shared protocol implementation: `src/rystemdb-protocol` (spec and wire conformance [#24](https://github.com/KeyserDSoze/RystemDatabase/issues/24)).

**As of first scaffolding:** only the Rust client crate exposes frame types; these packages do **not** yet provide working remote connections. Remote server and TLS/auth are [#25](https://github.com/KeyserDSoze/RystemDatabase/issues/25). Do not advertise any SDK as production-ready.

## Required definition of done
- Matching Rust/.NET/TypeScript binary golden vectors, malformed-frame tests and protocol-version negotiation
- Real TCP/TLS server end-to-end CRUD tests and authentication
- Backpressure, request correlation, cancellation, timeouts and retry semantics
- Cargo / NuGet / npm package builds and verified install tests across supported platforms

This code is developed directly on `main`, with RED/GREEN TDD and GitHub Issues.
