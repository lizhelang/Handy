#!/bin/bash
set -euo pipefail

# 复用本工作树已有依赖，不构建/启动 Handy，也不安装候选。
repo_root=$(cd "$(dirname "$0")/../../.." && pwd)
deps_dir=${UNIFIED_TARGET_DEPS_DIR:-"$repo_root/src-tauri/target/debug/deps"}
toolchain=${UNIFIED_TARGET_TOOLCHAIN:-1.96.0}
probe_dir=$(mktemp -d /tmp/handy-unified-target.XXXXXX)
extern_args=()
for crate in objc2 objc2_app_kit objc2_foundation block2; do
  matches=("$deps_dir"/lib"$crate"-*.rlib)
  if [[ ${#matches[@]} -ne 1 || ! -f "${matches[0]}" ]]; then
    echo "Expected exactly one compiled $crate rlib in $deps_dir; select an isolated dependency directory." >&2
    exit 2
  fi
  extern_args+=(--extern "$crate=${matches[0]}")
done
source_file="$repo_root/macos/InputiaInputMethod/Tools/UnifiedTargetProbe.rs"
rustup run "$toolchain" rustc --edition=2021 -D warnings -L "dependency=$deps_dir" \
  "${extern_args[@]}" "$source_file" -o "$probe_dir/metadata"
"$probe_dir/metadata"
rustup run "$toolchain" rustc --edition=2021 -D warnings --test -L "dependency=$deps_dir" \
  "${extern_args[@]}" "$source_file" -o "$probe_dir/tests"
"$probe_dir/tests"
echo "evidence_executables=$probe_dir"
