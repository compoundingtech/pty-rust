# pty-rust specification

## Status

Implemented for the session-activity slice. Broader Node parity remains tracked by repository issues.

## Scope

This specification defines the per-session actor, compatible registry metadata, and durable output-activity evidence. It does not define harness semantics, active/idle thresholds, agent orchestration, delivery policy, or authorization.

## Architecture

```text
portable-pty reader thread
          |
          v
DaemonMsg::PtyData(Vec<u8>)
          |
          v
single daemon actor ──> libghostty terminal ──> streaming clients
          |
          +── actor-owned last_output_at_ms
                  |
                  +── trailing-edge deadline (1s)
                  v
          <PTY_ROOT>/<name>.json
```

The actor owns libghostty's non-`Send` terminal, the PTY writer, client registry, and activity timestamp (A03, R02). Reader/client threads only send `DaemonMsg` values.

## Registry wire

`registry::SessionMetadata` is serialized with camelCase field names. Output evidence is additive:

```json
{
  "lastOutputAtMs": 1787761801896
}
```

The field is absent until nonempty output is observed and absent in older records. Its value is unix milliseconds. `registry::write_metadata` writes pretty JSON to `<name>.json.tmp` and atomically renames it over `<name>.json` (R01, R03, R04).

## Activity persistence

The actor loop maintains:

```rust
last_output_at_ms: Option<u64>
activity_persist_deadline: Option<Instant>
```

For each nonempty `DaemonMsg::PtyData(bytes)`:

1. set `last_output_at_ms = now_epoch_ms()`;
2. if no activity deadline exists, schedule `Instant::now() + 1s`;
3. process the same bytes through libghostty and client broadcast.

The actor receives with `recv_timeout` while a deadline exists. On timeout it reads current metadata, changes only `last_output_at_ms`, writes atomically, clears the deadline, and returns to the loop. Further chunks inside the window update memory but do not schedule or write again (R04, R05).

On `DaemonMsg::PtyExited(code)`, retained-session finalization captures the screen/tail and writes exit fields plus the latest in-memory `last_output_at_ms` in the same metadata replacement. Reaped sessions remove metadata and therefore retain no evidence. The pending timer cannot lose the final retained stamp because the actor exits after that synchronous write (R04).

pty-rust reports evidence only. Consumers own activity windows and composition with richer observations.

## Validation

| Requirement | Owning source | Executable evidence |
| --- | --- | --- |
| R01, R03 | `src/registry.rs` | `tests/registry_liveness.rs`, `tests/cli_e2e.rs` |
| R02 | `src/daemon.rs` | existing terminal/stream parity tests |
| R04, R05 | `src/daemon.rs`, `src/registry.rs` | `output_activity_stamp_appears_and_advances`, `post_exit_peek_returns_final_screen` |
| R06 | crate/test harness | `cargo build`, `cargo test --test cli_e2e`, `cargo test --test registry_liveness` |
