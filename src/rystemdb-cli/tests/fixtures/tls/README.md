# TLS test certificates — NOT TRUSTED

These files are **public, committed fixtures**, intentionally including the server private key. Generated 2026-11-21, expiring 2036-06-21, signed by an intentionally untrusted root. Only for TDD local transport; never deploy or use in production, do not add this CA to a system trust store.

The server certificate has SAN DNS:localhost, IP:127.0.0.1 and IP:::1. `rpc_tls.rs` copies the private key into an isolated temporary directory with mode 0600 on Unix. These fixtures have no secret or identity value.
