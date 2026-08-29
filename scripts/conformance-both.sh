#!/usr/bin/env bash
# Run the conformance suite against two pty binaries and print a side-by-side
# summary: one row per test file with pass/fail counts for each binary, then
# the tests whose outcome differs (the parity debt).
#
# Usage: scripts/conformance-both.sh [--node <bin>] [--rust <bin>] [--out <dir>] [test-file ...]
#   --node   the reference binary (default: `pty` on PATH)
#   --rust   the binary under port (default: target/debug/pty after `cargo build -p pty`)
#   --out    where per-file logs and `red.txt` go (default: target/conformance)
#   test-file  names like `integration_sync` (default: every tests/*.rs)
#
# Each side's raw log lives in <out>/<label>/<file>.log; <out>/red.txt lists the
# tests that pass on the first binary and fail on the second, with the first
# line of each failure message.
set -u
cd "$(dirname "$0")/.."

NODE_BIN="$(command -v pty || true)"
RUST_BIN="$PWD/target/debug/pty"
OUT="$PWD/target/conformance"
FILES=()
while [ $# -gt 0 ]; do
  case "$1" in
    --node) NODE_BIN="$2"; shift 2 ;;
    --rust) RUST_BIN="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) FILES+=("$1"); shift ;;
  esac
done
[ -x "$NODE_BIN" ] || { echo "node pty not found (use --node)"; exit 2; }
[ -x "$RUST_BIN" ] || { echo "rust pty not found at $RUST_BIN (cargo build -p pty, or --rust)"; exit 2; }
if [ ${#FILES[@]} -eq 0 ]; then
  for f in crates/pty-conformance/tests/*.rs; do
    FILES+=("$(basename "$f" .rs)")
  done
fi

cargo test -p pty-conformance --no-run -q 2>&1 | grep -v '^\s*$' | grep -iv 'warning' || true

run_side() {
  local label="$1" bin="$2"
  mkdir -p "$OUT/$label"
  for f in "${FILES[@]}"; do
    PTY_TEST_BIN="$bin" cargo test -p pty-conformance --test "$f" -- --test-threads=4 \
      > "$OUT/$label/$f.log" 2>&1 || true
  done
}

echo "node: $NODE_BIN"
echo "rust: $RUST_BIN"
echo "logs: $OUT"
run_side node "$NODE_BIN"
run_side rust "$RUST_BIN"

python3 - "$OUT" "${FILES[@]}" <<'PY'
import re, sys, os
out = sys.argv[1]
files = sys.argv[2:]
res = {}
def parse(path):
    tests = {}
    fails = {}
    if not os.path.exists(path):
        return tests, fails
    text = open(path, errors="replace").read()
    for m in re.finditer(r"^test (\S+) \.\.\. (ok|FAILED|ignored)", text, re.M):
        tests[m.group(1)] = m.group(2)
    # First line of each failure's stdout section (the panic message).
    for m in re.finditer(r"^---- (\S+) stdout ----\n(.*?)(?=^---- |\nfailures:|\Z)", text, re.M | re.S):
        body = m.group(2)
        # The panic location line is followed by the message; keep the
        # message's first line (or the location when there is none).
        first = ""
        where = ""
        for line in body.splitlines():
            line = line.strip()
            if not line or line.startswith("note: run with"):
                continue
            if re.match(r"thread '.*' (\(\d+\) )?panicked at ", line):
                where = line
                tail = line.split("panicked at ", 1)[1]
                rest = tail.split(":", 3)[3].strip() if tail.count(":") >= 3 else ""
                if rest:
                    first = rest
                    break
                continue
            first = line
            break
        msg = (first or where)[:200]
        # The rig's Out summary follows; the first stderr line usually names
        # the cause (an unknown command, a different message).
        if "--- stderr ---" in body:
            err = body.split("--- stderr ---", 1)[1]
            for line in err.splitlines():
                line = line.strip()
                if line and not line.startswith("note: run with"):
                    msg += " :: " + line[:160]
                    break
        fails[m.group(1)] = msg
    return tests, fails
rows = []
red = []
for f in files:
    nt, nf = parse(f"{out}/node/{f}.log")
    rt, rf = parse(f"{out}/rust/{f}.log")
    def counts(t):
        return sum(1 for v in t.values() if v == "ok"), sum(1 for v in t.values() if v == "FAILED")
    np_, nfail = counts(nt)
    rp, rfail = counts(rt)
    rows.append((f, np_, nfail, rp, rfail))
    for name, status in sorted(nt.items()):
        if status == "ok" and rt.get(name) == "FAILED":
            red.append(f"{f}::{name} -- {rf.get(name, '')}")
        if status == "FAILED":
            red.append(f"{f}::{name} -- NODE RED: {nf.get(name, '')}")
w = max(len(r[0]) for r in rows) if rows else 10
print()
print(f"{'file':<{w}}  {'node pass/fail':>14}  {'rust pass/fail':>14}")
print(f"{'-'*w}  {'-'*14}  {'-'*14}")
tn = [0, 0, 0, 0]
for f, a, b, c, d in rows:
    flag = "" if b == 0 and d == 0 else ("  <- rust" if b == 0 else "  <- NODE")
    print(f"{f:<{w}}  {a:>7}/{b:<6}  {c:>7}/{d:<6}{flag}")
    tn[0] += a; tn[1] += b; tn[2] += c; tn[3] += d
print(f"{'-'*w}  {'-'*14}  {'-'*14}")
print(f"{'total':<{w}}  {tn[0]:>7}/{tn[1]:<6}  {tn[2]:>7}/{tn[3]:<6}")
with open(f"{out}/red.txt", "w") as fh:
    fh.write("\n".join(red) + ("\n" if red else ""))
print(f"\n{len(red)} tests differ; see {out}/red.txt")
for line in red[:200]:
    print("  " + line)
PY
