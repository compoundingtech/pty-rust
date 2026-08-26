# pty-rust requirements

## Context

pty-rust is a Rust implementation of the compoundingtech/pty session protocol and registry contract. It owns a per-session daemon, PTY child, libghostty terminal state, and Unix-socket clients. These requirements define the durable constraints needed for Node/Rust compatibility and launcher-agnostic session activity evidence.

## Assumptions

- **A01 Unix runtime:** Supported hosts provide Unix PTYs, sockets, signals, process liveness probes, and atomic same-filesystem rename.
- **A02 Trusted user registry:** One registry belongs to one trusted OS user; filesystem permissions are the access boundary.
- **A03 Actor ownership:** The daemon actor is the single owner of terminal state. Reader/client threads communicate with it through typed messages.
- **A04 Universal output evidence:** Every successful PTY reader chunk reaches `DaemonMsg::PtyData` before terminal parsing and client broadcast. Recording when that happens adds no observer and carries no launcher or harness semantics.

## Acceptable tradeoffs

- **T01 Per-session daemon:** Each session pays for one independent actor/daemon in exchange for client-independent lifetime and failure isolation.
- **T02 Coalesced metadata:** Live output evidence may lag the newest chunk by at most one second to bound metadata writes; retained exit metadata must carry the final in-memory value.
- **T03 Pre-1.0 storage evolution:** Optional additive metadata fields may appear, while older records without them remain readable.

## Requirements

- **R01 Node-compatible session registry:** Stable ids own metadata, socket, pid, and retained-screen artifacts under `PTY_ROOT`; field names and omission behavior remain compatible with compoundingtech/pty where the surface is implemented.
- **R02 Ordered terminal ownership:** The daemon actor applies PTY output to libghostty before exposing derived screen state, and preserves output ordering for streaming clients.
- **R03 Atomic durable metadata:** Metadata writes serialize one complete JSON object to a sibling temporary file and atomically rename it over the session record. Optional unknown/additive fields survive supported read-modify-write paths.
- **R04 Durable output-activity evidence:** Session metadata exposes optional unix-millisecond `lastOutputAtMs`. The actor stamps every nonempty `PtyData` message, persists the newest stamp on a trailing-edge one-second deadline, and carries the final in-memory stamp into retained exit metadata before teardown. A new silent session omits the field. The field is evidence only: pty-rust does not classify active/idle, infer liveness, or authorize lifecycle/delivery behavior.
- **R05 Bounded write amplification:** A continuously chatty session performs at most one live activity metadata persist per one-second window. No filesystem work occurs per output chunk beyond updating actor-owned memory.
- **R06 Behavioral proof:** Real-process tests cover absence before output, a recent stamp after output, monotonic advancement after a later burst, and immediate output followed by exit retaining the final stamp. Build/test tooling must compile the daemon actor and registry on the pinned Rust/libghostty toolchain.
