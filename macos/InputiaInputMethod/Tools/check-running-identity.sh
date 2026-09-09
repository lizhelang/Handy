#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
# 缺少诊断入口时直接失败，绝不误入旧脚本的系统安装检查。
if ! grep -q -- '--running-identity' "$root/install-check.sh"; then
  echo 'runningIdentityRegression=false reason=missing-dynamic-check'
  exit 1
fi
fixture_root="$(mktemp -d /private/tmp/inputia-running-identity.XXXXXX)"
old_pid=''
new_pid=''
cleanup() {
  if [[ -n "$old_pid" ]]; then kill "$old_pid" 2>/dev/null || true; wait "$old_pid" 2>/dev/null || true; fi
  if [[ -n "$new_pid" ]]; then kill "$new_pid" 2>/dev/null || true; wait "$new_pid" 2>/dev/null || true; fi
  # 有意保留小型合成二进制供动态身份复核；不含用户数据或私钥。
}
trap cleanup EXIT
clang -DFIXTURE_VERSION=1 "$root/Tools/RunningIdentityFixture.c" -o "$fixture_root/host"
clang -DFIXTURE_VERSION=2 "$root/Tools/RunningIdentityFixture.c" -o "$fixture_root/new-host"
for binary in "$fixture_root/host" "$fixture_root/new-host"; do
  codesign --force --sign - --identifier com.inputia.running-identity-fixture "$binary"
done
"$fixture_root/host" & old_pid=$!
bash "$root/install-check.sh" --running-identity "$fixture_root/host" "$old_pid"
mv "$fixture_root/host" "$fixture_root/retired-host"
cp "$fixture_root/new-host" "$fixture_root/host"
if bash "$root/install-check.sh" --running-identity "$fixture_root/host" "$old_pid"; then
  echo 'runningIdentityRegression=false reason=accepted-retired-process'
  exit 1
fi
"$fixture_root/host" & new_pid=$!
bash "$root/install-check.sh" --running-identity "$fixture_root/host" "$new_pid"
if bash "$root/install-check.sh" --running-identity "$fixture_root/host" 0; then
  echo 'runningIdentityRegression=false reason=accepted-invalid-pid'
  exit 1
fi
echo "runningIdentityRegression=true oldRejected=true newAccepted=true fixture=$fixture_root"
