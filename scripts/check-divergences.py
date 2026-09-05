#!/usr/bin/env python3
"""Gate the conformance divergences against the recorded ledger.

`scripts/conformance-both.sh` writes `<out>/red.txt`: one line per test whose
result differs between the two binaries. Two shapes appear there.

    <file>::<test> -- <message>              passes on Node, FAILS on Rust
    <file>::<test> -- NODE RED: <message>    fails on Node

This script compares that list against `crates/pty-conformance/divergences.toml`
and fails when the two disagree, in EITHER direction:

  * an unrecorded divergence  -> a difference nobody has judged;
  * a recorded divergence that no longer happens -> a stale ledger entry.

The second half is the point. A ledger that only ever grows becomes a list of
claims nobody re-reads, and the entries outlive the reasons for them. Failing on
a stale entry forces the ledger to describe the present.
"""

import re
import sys
import tomllib
from pathlib import Path


def parse_red(path: Path) -> dict[str, str]:
    """test id -> "node" (fails on Node) or "rust" (passes Node, fails Rust)."""
    found = {}
    if not path.exists():
        return found
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        m = re.match(r"^(\S+) -- (NODE RED: )?(.*)$", line)
        if not m:
            print(f"check-divergences: cannot parse red.txt line: {line}", file=sys.stderr)
            return {"<unparsed>": "rust"}
        found[m.group(1)] = "node" if m.group(2) else "rust"
    return found


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    red = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "target/conformance/red.txt"
    ledger_path = root / "crates/pty-conformance/divergences.toml"

    found = parse_red(red)
    ledger = tomllib.loads(ledger_path.read_text()) if ledger_path.exists() else {}
    recorded = {d["test"]: d for d in ledger.get("divergence", [])}

    unrecorded = sorted(set(found) - set(recorded))
    stale = sorted(set(recorded) - set(found))
    wrong_side = sorted(
        t for t in set(found) & set(recorded) if recorded[t].get("side") != found[t]
    )

    print(f"conformance divergences: {len(found)} observed, {len(recorded)} recorded")

    if not (unrecorded or stale or wrong_side):
        for t in sorted(found):
            print(f"  recorded  {t}  ({found[t]}-red)")
        print("OK: every divergence is recorded, and every record is real.")
        return 0

    if unrecorded:
        print("\nUNRECORDED divergence — the two implementations differ and nobody has said why.")
        print("Fix the difference, or add an entry to crates/pty-conformance/divergences.toml")
        print("that states which side fails and why that is intended.")
        for t in unrecorded:
            print(f"  {t}  ({found[t]}-red)")

    if stale:
        print("\nSTALE ledger entry — recorded as an intended divergence, but the test now agrees.")
        print("Delete the entry. A ledger that keeps resolved entries stops describing the present.")
        for t in stale:
            print(f"  {t}  (recorded {recorded[t].get('side')}-red)")

    if wrong_side:
        print("\nWRONG SIDE — the divergence is real but fails on the other implementation now.")
        for t in wrong_side:
            print(f"  {t}  recorded {recorded[t].get('side')}-red, observed {found[t]}-red")

    return 1


if __name__ == "__main__":
    sys.exit(main())
