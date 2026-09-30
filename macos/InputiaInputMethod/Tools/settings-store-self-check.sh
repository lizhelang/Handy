#!/bin/bash
set -euo pipefail
task_tools="$(cd "$(dirname "$0")" && pwd -P)"
task_temp="$(mktemp -d "${TMPDIR:-/tmp}/inputia-settings-contract.XXXXXX")"
trap 'rm -rf "$task_temp"' EXIT
/usr/bin/swiftc -parse-as-library -target "$(uname -m)-apple-macos13.0" \
  "$task_tools/InputiaSettingsStoreSelfCheck.swift" \
  "$task_tools/../Sources/InputiaInputMethod/InputiaSettingsStore.swift" \
  -o "$task_temp/settings-store-self-check"
"$task_temp/settings-store-self-check"
