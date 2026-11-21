# Locking TDD log

Storage writer lock behavior is tracked in [issue #17](https://github.com/KeyserDSoze/RystemDatabase/issues/17).

RED acceptance tests are in `src/rystemdb-storage/tests/storage_contract.rs`: second same-process open, another process competing, and abrupt child process death releasing the lock. They must fail before any writer-lock implementation. The exact RED and GREEN CI links will be recorded in issue #17.

The writer lock is an advisory OS lock. It does not prevent arbitrary uncooperative processes from directly editing data files.
