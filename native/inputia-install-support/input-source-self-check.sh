#!/bin/bash
set -euo pipefail
task_root="$(cd "$(dirname "$0")" && pwd -P)"
task_temp="$(mktemp -d "${TMPDIR:-/tmp}/inputia-input-source.XXXXXX")"
trap 'rm -rf "$task_temp"' EXIT
/usr/bin/swiftc -parse-as-library -target "$(uname -m)-apple-macos13.0" \
  "$task_root/InputiaInstallSupport.swift" "$task_root/InputiaWriterQuiescence.swift" \
  "$task_root/InputiaInputSource.swift" "$task_root/InputSourceSelfCheck.swift" \
  -framework Carbon -framework Security -framework CryptoKit -o "$task_temp/self-check"
"$task_temp/self-check"
