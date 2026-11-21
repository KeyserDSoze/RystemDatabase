# TLS test certificates — NOT TRUSTED

These files are **public, committed fixtures**, intentionally including the server private key. Generated 2026-11-21, expiring 2036-06-21, signed by an intentionally untrusted root. Only for TDD local transport; never deploy or use in production, do not add this CA to a system trust store.

The server certificate has SAN DNS:localhost, IP:127.0.0.1 and IP:::1. `rpc_tls.rs` copies the private key into an isolated temporary directory with mode 0600 on Unix. These fixtures have no secret or identity value.

The additional `validity-root-ca.pem`, `validity-server-key.pem`, `expired-server-cert.pem` (expired 2020-01-01) and `future-server-cert.pem` (valid from 2035-01-01) are also **public test-only fixtures**. The CA is valid 2010–2045, so failed chain validation must be due to the leaf's date, not the test root. The private test server key is intentionally checked into GitHub; it must never protect real data.
