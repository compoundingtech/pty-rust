# 0005 — A Rust write leaves `recovery.metadataRevision` behind

**Status:** accepted

**Node behavior.** A Node daemon may write a `recovery{}` object into a
session's metadata, and one of its fields is `metadataRevision`. Node bumps
that field on every metadata write, so it always names the revision the
recovery data was taken from (`src/sessions.ts`, the recovery capability).
`pty recover` reads it and refuses a snapshot whose revision no longer
matches the record.

**Rust behavior.** The Rust pty never writes `recovery{}` and never reads
it, but it preserves the whole object verbatim when it rewrites a record —
`pty tag`, `pty rename`, `pty metadata patch` and the daemon's own writes
all copy unknown fields through unchanged (docs/parity.md §11). So after a
Rust write, `recovery.metadataRevision` still names the revision from before
that write.

**Why.** Preserving an unknown object verbatim is the rule that keeps a
mixed registry safe: the Rust side must not delete a field it does not
understand. Bumping `metadataRevision` would mean implementing the recovery
capability, which is deferred (docs/parity.md §12), and guessing at the
semantics of a field this implementation does not maintain would be worse
than leaving it alone.

**Client effect.** Only a caller that runs the **Node** `pty recover` on a
root where a **Rust** binary has written to the same session. That recover
may reject the snapshot because the stale revision no longer matches. Every
other operation is unaffected, and a root that only the Rust binary writes
never grows a `recovery{}` object at all. No program in this network calls
`recover`.

**Test.** None. The difference needs the Node recovery capability on both
sides to observe, and that capability is deferred rather than ported, so
there is nothing to gate. `crates/pty-conformance/tests/metadata_events.rs`
covers that unknown fields survive a Rust rewrite, which is the half of the
behaviour this repository owns.

**Migration / negotiation.** None. If `recover` is ever ported, this record
is superseded by one that says how the Rust side maintains the field.
