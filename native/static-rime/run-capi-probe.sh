#!/bin/bash
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "$0")" && pwd -P)"
REPOSITORY="$(cd "$ROOT/../.." && pwd -P)"
ARCH="$(uname -m)"
export MACOSX_DEPLOYMENT_TARGET=13.0
export INPUTIA_STATIC_RIME_DIR="$ROOT/artifacts/output/$ARCH"
: "${INPUTIA_RIME_SHARED_DATA_DIR:?explicit candidate RimeData is required}"
export CARGO_TARGET_DIR="$ROOT/artifacts/capi-target-$ARCH"
cargo +1.96.0 build --release --manifest-path "$REPOSITORY/crates/inputia-capi/Cargo.toml" --features bundled-static-rime
OUTPUT="$ROOT/artifacts/output/$ARCH"
PROBE="$OUTPUT/CAPIStaticProbe"
/usr/bin/swiftc "$ROOT/probe/capi.swift" -parse-as-library -target "$ARCH-apple-macos13.0" \
  -Xlinker -force_load -Xlinker "$CARGO_TARGET_DIR/release/libinputia_capi.a" \
  -lc++ -framework Foundation -o "$PROBE"
/usr/bin/codesign --force --sign - --options runtime "$PROBE"
/usr/bin/codesign --verify --strict --verbose=2 "$PROBE"
/usr/bin/codesign -dv --verbose=4 "$PROBE" 2> "$OUTPUT/capi-probe-codesign.txt"
/usr/bin/otool -L "$PROBE" > "$OUTPUT/capi-probe-dependencies.txt"
RUN="$(mktemp -d "$ROOT/artifacts/capi-probe-run.XXXXXX")"
"$PROBE" "$INPUTIA_RIME_SHARED_DATA_DIR" "$RUN" 2>&1 | tee "$RUN/probe.log"
echo "capiStaticProbeEvidence=$RUN/probe.log"
