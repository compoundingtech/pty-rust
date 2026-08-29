# Open items for the maintainer (batched; none gate the build)

- 2026-08-29 lane A: a Rust `tag`/`rename` on a session written by a Node daemon preserves `recovery{}`
  verbatim, so Node's `recovery.metadataRevision` goes stale for that session. `pty recover` (deferred)
  may then reject it. Only matters if someone runs the Node `recover` on a mixed root. Decision record
  pending; recommended: accept, document in docs/parity.md §12.
- 2026-08-29 lane C: two Node quirks not reproduced on purpose: `queryStats` fails immediately when the
  daemon closes without STATUS (Node waits the full 2 s); `peek -f` on a plain close returns exit 0 (Node
  hangs). Both are strictly better for callers.
- 2026-08-29 lane D: nix is not installed on the Linux build host. The flake was built with nix-portable (store under
  ~/.local/state/pty-rust-parity/np). Hashes verified; hermeticity of the main build not enforced there.
  A real `nix build` on a proper nix install is the remaining proof.
- 2026-08-29 lane D: the check phase in the flake runs the whole workspace test suite (needed a short TMPDIR
  and bashInteractive); conformance checks come later.
