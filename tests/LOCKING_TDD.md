# Locking TDD log

Storage writer lock behavior is tracked in [issue #17](https://github.com/KeyserDSoze/RystemDatabase/issues/17).

RED acceptance tests are in `src/rystemdb-storage/tests/storage_contract.rs`: second same-process open, another process competing, and abrupt child process death releasing the lock. They were committed RED-first as [a80c642](https://github.com/KeyserDSoze/RystemDatabase/commit/a80c642a6a9e0cab74a676cfc1f93e2cafa9dfce) and failed in [CI 44913916783](https://github.com/KeyserDSoze/RystemDatabase/actions/runs/44913916783): 3 behavioral failures, 14 existing assertions passing on Linux. GREEN CI for the implementation is tracked in issue #17.

The writer lock is an advisory OS lock. It does not prevent arbitrary uncooperative processes from directly editing data files.
