#!/bin/bash
set -euo pipefail
task_root="$(cd "$(dirname "$0")" && pwd -P)"
task_temp="$(mktemp -d "${TMPDIR:-/tmp}/inputia-install-security.XXXXXX")"
trap 'rm -rf "$task_temp"' EXIT
/usr/bin/swiftc -parse-as-library -target "$(uname -m)-apple-macos13.0" \
  "$task_root/InputiaInstallSupport.swift" "$task_root/InstallSupportSelfCheck.swift" \
  -framework Security -framework CryptoKit -o "$task_temp/self-check"
"$task_temp/self-check"
