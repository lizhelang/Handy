#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$ROOT_DIR/../.." && pwd)"
RUST_TOOLCHAIN="${INPUTIA_RUST_TOOLCHAIN:-1.96.0}"
HOST_APP="$ROOT_DIR/build/InputiaInputMethod.app"
RIME_USER_DATA_DIR=""

schema="${INPUTIA_RIME_LATENCY_SCHEMA:-double_pinyin}"
input="${INPUTIA_RIME_LATENCY_INPUT:-mlle}"
iterations="${INPUTIA_RIME_LATENCY_ITERATIONS:-20}"
max_incremental_ms="${INPUTIA_RIME_LATENCY_MAX_INCREMENTAL_MS:-1000}"
min_speedup="${INPUTIA_RIME_LATENCY_MIN_SPEEDUP:-1.2}"

run_rust() {
  if command -v rustup >/dev/null 2>&1; then
    rustup run "$RUST_TOOLCHAIN" "$@"
  else
    "$@"
  fi
}

cleanup() {
  if [[ -n "$RIME_USER_DATA_DIR" ]]; then
    /bin/rm -rf "$RIME_USER_DATA_DIR" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

RIME_USER_DATA_DIR="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-rime-latency.XXXXXX")"

echo "rimeLatencyTouchesMenuBar=false"
echo "rimeLatencyOpensGUI=false"
echo "rimeLatencyChangesSystemInputSource=false"
echo "rimeLatencyChecksNotarization=false"
echo "rimeLatencySchema=$schema"
echo "rimeLatencyInput=$input"
echo "rimeLatencyIterations=$iterations"
echo "rimeLatencyMaxIncrementalMs=$max_incremental_ms"
echo "rimeLatencyMinSpeedup=$min_speedup"

probe_output="$(
  INPUTIA_RIME_SHARED_DATA_DIR="$HOST_APP/Contents/Resources/RimeData" \
    INPUTIA_RIME_USER_DATA_DIR="$RIME_USER_DATA_DIR" \
    run_rust cargo run --quiet --manifest-path "$REPO_ROOT/crates/inputia-rime/Cargo.toml" \
      --example persistent_session_probe -- "$schema" "$input" "$iterations"
)"
printf '%s\n' "$probe_output" | /usr/bin/sed 's/^/rimeLatencyProbe: /'

PROBE_OUTPUT="$probe_output" /usr/bin/python3 - "$max_incremental_ms" "$min_speedup" <<'PY'
import os
import sys

max_incremental_ms = float(sys.argv[1])
min_speedup = float(sys.argv[2])

values = {}
for line in os.environ["PROBE_OUTPUT"].splitlines():
    if "=" in line:
        key, value = line.split("=", 1)
        values[key] = value

required = [
    "cold_first",
    "incremental_first",
    "cold_ms",
    "incremental_ms",
    "speedup",
]
for key in required:
    if key not in values:
        print(f"rimeLatencySelfCheck=false reason=missing-{key}")
        raise SystemExit(1)

cold_first = values["cold_first"]
incremental_first = values["incremental_first"]
if cold_first == "<none>" or incremental_first == "<none>":
    print("rimeLatencySelfCheck=false reason=missing-candidate")
    raise SystemExit(1)
if cold_first != incremental_first:
    print(
        "rimeLatencySelfCheck=false "
        f"reason=first-candidate-mismatch cold={cold_first} incremental={incremental_first}"
    )
    raise SystemExit(1)

cold_ms = float(values["cold_ms"])
incremental_ms = float(values["incremental_ms"])
speedup = float(values["speedup"].rstrip("x"))

print(f"rimeLatencyColdMs={cold_ms:.3f}")
print(f"rimeLatencyIncrementalMs={incremental_ms:.3f}")
print(f"rimeLatencySpeedup={speedup:.2f}")

if incremental_ms > max_incremental_ms:
    print(
        "rimeLatencySelfCheck=false "
        f"reason=incremental-too-slow actual={incremental_ms:.3f} max={max_incremental_ms:.3f}"
    )
    raise SystemExit(1)
if speedup < min_speedup:
    print(
        "rimeLatencySelfCheck=false "
        f"reason=incremental-speedup-too-low actual={speedup:.2f} min={min_speedup:.2f}"
    )
    raise SystemExit(1)
if incremental_ms > cold_ms:
    print(
        "rimeLatencySelfCheck=false "
        f"reason=incremental-slower-than-cold cold={cold_ms:.3f} incremental={incremental_ms:.3f}"
    )
    raise SystemExit(1)

print("rimeLatencySelfCheck=true")
PY
