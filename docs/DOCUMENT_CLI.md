# Document CLI — experimental contract (Issue #21)

The native Rust `rystemdb` executable works on **local** binary document segments using `rystemdb-storage`. It is not a network database service.

## Basic usage

```sh
printf '{"name":"Ada","items":[{"sku":"A12","qty":2}],"shipping":{"city":"Roma"}}' |
  rystemdb document put --data ./rystem-data --collection orders --id order_1 --json -

rystemdb document get --data ./rystem-data --collection orders --id order_1
rystemdb document delete --data ./rystem-data --collection orders --id order_1
```

`--json -` reads UTF-8 JSON from stdin. `--json ./input.json` reads an on-disk JSON file. A document's root **must be a JSON object**. The CLI reports errors and exits non-zero for invalid requests or missing IDs. Key naming: `--collection` 1–128 ASCII alphanumerics, underscores or hyphens; `--id` nonempty UTF-8, combined key `collection/id` at most 1024 bytes and no control characters. Slash is allowed in the ID. The ID is scoped to the collection.

The document is persisted as a self-describing typed binary `RDB\0` payload in checksummed append-only `RYSF` frames, **not as JSON text on disk**. Each CLI invocation acquires an OS advisory exclusive writer lock for the database directory; the lock is released on normal exit or process death. PUT overwrites logically by appending a new version, DELETE appends a tombstone. Restart rebuilds the ID index by replaying segment files.

## JSON scalar semantics

- `null`, booleans, UTF-8 strings, nested objects, heterogeneous arrays are preserved.
- JSON integers must fit **signed 64-bit** (`i64`); larger integer literals are rejected rather than rounded.
- Decimal and exponent number literals are represented as finite **IEEE-754 f64** (not decimal128); use strings for exact monetary values until a native Decimal type is implemented.
- Missing fields and explicit `null` remain distinct.
- The maximum JSON request body is currently 64 MiB. The binary record codec has its own limits, including nesting depth.
- `get` writes compact JSON to stdout, ordering object keys lexicographically by default.

## Explicit typed JSON for references and bytes

By default, **all** JSON objects are parsed as plain maps. To request native `Value::Reference` or `Value::Bytes`, the caller must explicitly pass `--typed-json` to `document put`:

```sh
cat <<'JSON' | rystemdb document put --data ./rystem-data --collection orders --id order_2 --json - --typed-json
{
  "customer": {
    "$rystem": {
      "version": 1,
      "type": "reference",
      "collection": "users",
      "id": "u42"
    }
  },
  "payload": {
    "$rystem": {
      "version": 1,
      "type": "bytes",
      "hex": "0080ff"
    }
  }
}
JSON
```

When getting a document, native references and bytes are emitted using the same tagged JSON wrapper. To import that JSON preserving its native types, use `--typed-json` on PUT. The reference tag **does not yet enforce foreign-key integrity**; see Issue #4.

If a literal JSON object itself consists of a single `"$rystem"` field and must remain an **ordinary object** in typed mode, escape it explicitly:

```json
{"record":{"$rystem":{"version":1,"type":"object","value":{"$rystem":{"version":1,"type":"reference","collection":"users","id":"u42"}}}}}
```

The `"object"` tag forces the wrapped `value` to be read as an ordinary map (including its nested keys), rather than reinterpreted as a native reference. With `--typed-json` omitted, even reference-looking maps always remain plain JSON objects.

## Operational scope

This is an experimental local CLI, not yet an ACID database with SQL or relations. Concurrent independent writers are rejected by a cooperative OS lock; a read CLI also opens a writer-protected store for replay. No multi-document transactions, MVCC, secondary indexes, auth, encryption, or backup/restore guarantees. Physical power-loss durability is not yet certified (Issues #2 / #17). Untrusted users must not be given direct file-system access to database directories.

The `serve` command only exposes `/healthz` and `/readyz`; no document API exists over HTTP. To run in Docker, mount a persistent `/data` volume and invoke the CLI directly.

## Tests / release gate

`src/rystemdb-cli/tests/document_cli.rs` launches distinct CLI processes for PUT, GET, overwrite, DELETE, null vs missing, UTF-8, nested types, typed reference/byte round-trips with the embedded Rust library, invalid input and lock conflict. All cargo tests, feature-gated storage crash suites, npm tests and a persistent Docker volume smoke test must pass on `main` before any document CLI release is marked ready.
