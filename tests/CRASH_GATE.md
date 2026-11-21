# Crash test gates

The initial writer-lock tests verify OS-level exclusivity and automatic release on process termination. The upcoming crash-injection suite must still test faults at every storage write/sync/rollover boundary, with cross-OS power-loss semantics and real persistent index publication.
