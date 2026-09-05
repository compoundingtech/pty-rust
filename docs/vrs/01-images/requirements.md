# Terminal images — Requirements

## Context

This node defines the durable terminal-image contract for pty-rust. The implementation uses the Kitty graphics protocol and libghostty. The rationale for the protocol and replay shape remains in [decision 0012](../../decisions/0012-kitty-graphics-replay.md).

## Assumptions

- **PTY.IMG-A01 Typed terminal ownership:** The terminal actor is the single owner of parsed image state. Clients read typed image descriptions, bytes, placements, and generations rather than untrusted escape sequences.
- **PTY.IMG-A02 Cell-relative composition:** An embedding client composes terminal images in character-cell coordinates and needs the same effective cell metrics that the terminal used.

## Constraints

- **PTY.IMG-C01 Library enumeration:** libghostty does not enumerate an image that was transmitted but never placed; it exposes image lookup by identifier.
- **PTY.IMG-C02 Pixel formats:** Kitty has no `f=` value for grayscale pixels, so a grayscale raw-pixel transmission cannot be represented as a typed replay image without conversion.
- **PTY.IMG-C03 Scrolled direct placements:** A cursor-positioned placement that is entirely above the active area cannot be re-emitted because restoring its position requires a cursor cell inside the active area.
- **PTY.IMG-C04 Protocol scope:** SIXEL and iTerm2 output do not provide the typed placement contract required for client-side reprojection and are outside this node.
- **PTY.IMG-C05 Cross-runtime conformance:** The Node implementation has no equivalent typed image state, so graphics behavior has no Node/Rust conformance pair.

## Acceptable Tradeoffs

- **PTY.IMG-T01 Complete bounded replay:** A session may retain up to 32 MiB of image state, and a replay may carry all retained bytes. The limit is a cap, not an eager allocation.
- **PTY.IMG-T02 Latest cell declaration wins:** Cell metrics use the newest client declaration rather than multi-client negotiation.

## Requirements

### Must bound accepted image state

- **PTY.IMG-R01 Opt-in bounded storage:** Image storage is disabled until an owner enables it. The effective storage limit must not exceed 32 MiB and must be observable through the typed terminal API.
- **PTY.IMG-R08 Inline-only transmission:** The terminal must accept image bytes only from inline transmissions. File, temporary-file, and shared-memory media must remain disabled so a child cannot name an owner-unapproved path.
- **PTY.IMG-R09 Bounded PNG normalization:** PNG (`f=100`) input must be bounded before decoding and normalized to 8-bit RGBA before it enters retained image storage.

### Must survive replay

- **PTY.IMG-R02 Complete late-client replay:** Every retained image reported for a placement must be carried in full by ATTACH and PEEK replay. The replay must not impose a smaller byte cap than the reported retained state.
- **PTY.IMG-R04 Crop fidelity:** Replay must preserve the resolved, image-bounded source rectangle and offsets for each placement.
- **PTY.IMG-R06 Image-free compatibility:** A session that has accepted no image must retain the pre-image replay byte shape; its image replay block is empty.
- **PTY.IMG-R07 Screen and reset lifecycle:** Normal and alternate screens must retain separate image state. Terminal reset must preserve the configured graphics capability needed to replay retained state into its owning screen.

### Must support cell-relative composition

- **PTY.IMG-R03 Window-relative placements:** A graphics read and a grid read taken at the same scroll offset must describe the same terminal window and align cell for cell. Virtual placements follow bounded placeholder cells; direct placements use their screen-space rectangle. Placements outside the requested window must not be reported as visible.
- **PTY.IMG-R05 Declared cell metrics:** ATTACH and RESIZE may append backward-compatible cell width and height fields. An absent declaration must stay explicit and use a deterministic 8×16 fallback. Explicit placement coordinates must not move when cell metrics change.
- **PTY.IMG-R10 Observable content generation:** Image-content changes must advance an observable generation used to fence cached pixels. Scroll and resize may reproject placements without advancing the content generation.
