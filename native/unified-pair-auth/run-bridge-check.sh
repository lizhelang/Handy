#!/bin/bash
set -euo pipefail
source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
bridge_dir="$(mktemp -d /tmp/uipb-build.XXXXXX)"
bridge_dir="$(cd -P "$bridge_dir" && pwd)"
sdk="$(xcrun --sdk macosx --show-sdk-path)"
swiftc="$(xcrun --find swiftc)"
swift_runtime="$(dirname "$(dirname "$swiftc")")/lib/swift/macosx"
architecture="$(uname -m)"
swift_target="$architecture-apple-macosx13.0"
case "$architecture" in
  arm64) rust_target=aarch64-apple-darwin ;;
  x86_64) rust_target=x86_64-apple-darwin ;;
  *) exit 2 ;;
esac
"$swiftc" -warnings-as-errors -parse-as-library -target "$swift_target" -sdk "$sdk" \
  "$source_dir/UnifiedPairAuth.swift" "$source_dir/PairAuthTool.swift" \
  -framework Security -o "$bridge_dir/BuildTool"
"$bridge_dir/BuildTool" keygen "$bridge_dir/build-private.x963" "$bridge_dir/public.x963"
# 私钥只在此专用 0700 临时目录的 0600 文件；无 Keychain import。
trap 'if [[ -f "$bridge_dir/build-private.x963" ]]; then rm -- "$bridge_dir/build-private.x963"; fi' EXIT
"$swiftc" -warnings-as-errors -parse-as-library -whole-module-optimization -O \
  -target "$swift_target" -sdk "$sdk" -module-name UnifiedPairAuthNative \
  -import-objc-header "$source_dir/PairAuthBridge.h" -emit-object \
  "$source_dir/UnifiedPairAuth.swift" "$source_dir/PairAuthBridge.swift" -o "$bridge_dir/bridge.o"
libtool -static -o "$bridge_dir/libunified_pair_auth.a" "$bridge_dir/bridge.o"
export UIPA_FIXTURE_PUBLIC_KEY="$bridge_dir/public.x963"
export MACOSX_DEPLOYMENT_TARGET=13.0
rust_args=(--edition=2021 --target "$rust_target" -D warnings --check-cfg 'cfg(test)' --check-cfg 'cfg(pair_host_fixture)' --check-cfg 'cfg(unified_paired_build)' \
  -L "native=$bridge_dir" -L "native=$swift_runtime" -L "native=$sdk/usr/lib/swift" \
  -l static=unified_pair_auth -l framework=Foundation -l framework=Security -C link-arg=-Wl,-rpath,/usr/lib/swift)
rustc "${rust_args[@]}" "$source_dir/PairAuthBridgeCheck.rs" -o "$bridge_dir/HandyFixture"
rustc "${rust_args[@]}" --cfg pair_host_fixture "$source_dir/PairAuthBridgeCheck.rs" -o "$bridge_dir/HostFixture"
cp "$bridge_dir/HostFixture" "$bridge_dir/WeakHostFixture"
codesign --force --sign - --options runtime --identifier com.inputia.bridge.Handy "$bridge_dir/HandyFixture"
codesign --force --sign - --options runtime --identifier com.inputia.bridge.Host "$bridge_dir/HostFixture"
codesign --force --sign - --identifier com.inputia.bridge.Host "$bridge_dir/WeakHostFixture"
"$bridge_dir/BuildTool" bridge-fixture-manifest "$bridge_dir/build-private.x963" \
  "$bridge_dir/HandyFixture" "$bridge_dir/HostFixture" "$bridge_dir/WeakHostFixture" "$bridge_dir/pair.json"
"$bridge_dir/HandyFixture" self-check "$bridge_dir/pair.json" "$bridge_dir/HostFixture" "$bridge_dir/WeakHostFixture"
# 被manifest明确允许的CDHash也不能借runtime例外关闭可执行页保护。
# 只在本次隔离目录重签测试进程，绝不用于真实候选app。
cp "$bridge_dir/HostFixture" "$bridge_dir/UnsafePageHostFixture"
codesign --force --sign - --options runtime --identifier com.inputia.bridge.Host \
  --entitlements "$source_dir/UnsafePageProtectionFixture.entitlements" "$bridge_dir/UnsafePageHostFixture"
"$bridge_dir/BuildTool" bridge-fixture-manifest "$bridge_dir/build-private.x963" \
  "$bridge_dir/HandyFixture" "$bridge_dir/HostFixture" "$bridge_dir/UnsafePageHostFixture" "$bridge_dir/unsafe-page-pair.json"
"$bridge_dir/HandyFixture" self-check "$bridge_dir/unsafe-page-pair.json" "$bridge_dir/HostFixture" "$bridge_dir/UnsafePageHostFixture"
printf 'allowlisted_disabled_executable_page_protection_rejected=true\n'
printf 'rust_swift_bridge_directory=%s\n' "$bridge_dir"

"$bridge_dir/BuildTool" bridge-release-fixture-manifest "$bridge_dir/build-private.x963" \
  "$bridge_dir/HandyFixture" "$bridge_dir/HostFixture" "$bridge_dir/WeakHostFixture" "$bridge_dir/release-pair.json"
"$bridge_dir/HandyFixture" release-self-check "$bridge_dir/release-pair.json" "$bridge_dir/HostFixture" "$bridge_dir/WeakHostFixture"
"$bridge_dir/BuildTool" bridge-release-fixture-manifest "$bridge_dir/build-private.x963" \
  "$bridge_dir/HandyFixture" "$bridge_dir/HostFixture" "$bridge_dir/UnsafePageHostFixture" "$bridge_dir/release-unsafe-page-pair.json"
"$bridge_dir/HandyFixture" release-self-check "$bridge_dir/release-unsafe-page-pair.json" "$bridge_dir/HostFixture" "$bridge_dir/UnsafePageHostFixture"

# 真实 Python CLI -> Security 签名 -> 编译期 Swift 信任根验签；两个版本均覆盖。
repo_dir="$(cd "$source_dir/../.." && pwd)"
python3 "$repo_dir/scripts/inputia_release.py" prepare --mode local --output-dir "$bridge_dir/release-context" > "$bridge_dir/prepare-report.json"
for format in v1 v2; do
  if [[ "$format" == v1 ]]; then
    binding_args=(--run-id bridge-cli-fixture)
  else
    binding_args=(--release-context "$bridge_dir/release-context/build-context.json")
  fi
  /usr/bin/python3 "$source_dir/build_trust.py" "${binding_args[@]}" \
    --public-key "$bridge_dir/public.x963" --metadata "$bridge_dir/$format-build.json"
  /usr/bin/python3 "$source_dir/build_trust.py" "${binding_args[@]}" \
    --metadata "$bridge_dir/$format-build.json" --emit swift > "$bridge_dir/$format-trust.swift"
  /usr/bin/python3 "$source_dir/build_trust.py" "${binding_args[@]}" --metadata "$bridge_dir/$format-build.json" \
    --sign-pair --handy "$bridge_dir/HandyFixture" --inputia "$bridge_dir/HostFixture" \
    --build-tool "$bridge_dir/BuildTool" --private-key "$bridge_dir/build-private.x963" --manifest "$bridge_dir/$format-cli-pair.json"
  "$swiftc" -warnings-as-errors -parse-as-library -target "$swift_target" -sdk "$sdk" \
    "$source_dir/UnifiedPairAuth.swift" "$source_dir/GeneratedTrustCheck.swift" "$bridge_dir/$format-trust.swift" \
    -framework Security -o "$bridge_dir/$format-GeneratedTrustCheck"
  "$bridge_dir/$format-GeneratedTrustCheck" "$bridge_dir/$format-cli-pair.json"
done

# .app 根路径也走真实动态代码查询，覆盖 Security 对 bundle 与裸 Mach-O 的不同返回。
for role in Handy Host WeakHost; do
  mkdir -p "$bridge_dir/$role.app/Contents/MacOS"
  binary_name=HostFixture
  identifier=com.inputia.bridge.Host
  if [[ "$role" == Handy ]]; then binary_name=HandyFixture; identifier=com.inputia.bridge.Handy; fi
  cp "$bridge_dir/${role}Fixture" "$bridge_dir/$role.app/Contents/MacOS/$binary_name"
  cat > "$bridge_dir/$role.app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>$identifier</string><key>CFBundleExecutable</key><string>$binary_name</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>
PLIST
  if [[ "$role" == WeakHost ]]; then
    codesign --force --sign - --identifier "$identifier" "$bridge_dir/$role.app"
  else
    codesign --force --sign - --options runtime --identifier "$identifier" "$bridge_dir/$role.app"
  fi
done
"$bridge_dir/BuildTool" bridge-release-fixture-manifest "$bridge_dir/build-private.x963" \
  "$bridge_dir/Handy.app" "$bridge_dir/Host.app" "$bridge_dir/WeakHost.app" "$bridge_dir/bundle-pair.json"
"$bridge_dir/Handy.app/Contents/MacOS/HandyFixture" release-self-check "$bridge_dir/bundle-pair.json" \
  "$bridge_dir/Host.app/Contents/MacOS/HostFixture" "$bridge_dir/WeakHost.app/Contents/MacOS/HostFixture"
