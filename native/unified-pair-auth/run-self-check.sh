#!/bin/bash
set -euo pipefail

source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
build_dir="$(mktemp -d /tmp/uipa-build.XXXXXX)"
# 保留唯一临时构建目录作为可复核证据；自检内部的密钥仅在内存。
swiftc -warnings-as-errors -target arm64-apple-macosx13.0 -parse-as-library \
  "$source_dir/UnifiedPairAuth.swift" "$source_dir/PairAuthTool.swift" \
  -framework Security -o "$build_dir/HandyFixture"
cp "$build_dir/HandyFixture" "$build_dir/HostFixture"
swiftc -warnings-as-errors -target arm64-apple-macosx13.0 -parse-as-library \
  -D SYNTHETIC_ROGUE "$source_dir/UnifiedPairAuth.swift" "$source_dir/PairAuthTool.swift" \
  -framework Security -o "$build_dir/RogueFixture"
codesign --force --sign - --identifier com.inputia.synthetic.Handy "$build_dir/HandyFixture"
codesign --force --sign - --identifier com.inputia.synthetic.Host "$build_dir/HostFixture"
# 相同 Host identifier、不同机器码/CDHash，不能因为同 UID 和同 identifier 被接受。
codesign --force --sign - --identifier com.inputia.synthetic.Host "$build_dir/RogueFixture"
"$build_dir/HandyFixture" self-check "$build_dir/HostFixture" "$build_dir/RogueFixture"
printf 'synthetic_build_directory=%s\n' "$build_dir"

swiftc -warnings-as-errors -target "$(uname -m)-apple-macosx13.0" -parse-as-library \
  "$source_dir/UnifiedPairAuth.swift" "$source_dir/ReleasePairAuthCheck.swift" \
  -framework Security -o "$build_dir/ReleaseContractCheck"
"$build_dir/ReleaseContractCheck" "$source_dir/fixtures"
