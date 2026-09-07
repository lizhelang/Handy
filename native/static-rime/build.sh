#!/bin/bash
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "$0")" && pwd -P)"
ARTIFACTS="$ROOT/artifacts"
ARCH="${STATIC_RIME_ARCH:-$(uname -m)}"
case "$ARCH" in arm64|x86_64) ;; *) echo "unsupported static Rime architecture" >&2; exit 2 ;; esac
if [[ -L "$ARTIFACTS" ]]; then echo "artifacts cannot be a symbolic link" >&2; exit 2; fi
export MACOSX_DEPLOYMENT_TARGET=13.0
OUTPUT="$ARTIFACTS/output/$ARCH"
if [[ "${1:-}" == "--verify-only" ]]; then
  exec /usr/bin/python3 "$ROOT/verify_artifacts.py" "$ARCH" --verify-only
fi
/usr/bin/python3 "$ROOT/fetch_sources.py"
SNAPSHOT="$ARTIFACTS/source-snapshot.json"
snapshot_field() {
  /usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$SNAPSHOT" "$1"
}
SOURCE="$(snapshot_field source_root)"
DEPS="$(snapshot_field deps_root)"
RELEASE="$(snapshot_field release_root)"
LICENSES="$(snapshot_field licenses_root)"
BOOST="$(snapshot_field boost_root)"
GENERATION="$(snapshot_field generation_id)"
BUILD="$ARTIFACTS/build-$ARCH-$GENERATION"
SDK="$(/usr/bin/xcrun --sdk macosx --show-sdk-path)"
mkdir -p "$OUTPUT/lib" "$OUTPUT/include" "$OUTPUT/licenses"
cp "$SNAPSHOT" "$OUTPUT/source-snapshot.json"
/usr/bin/python3 "$ROOT/fetch_sources.py" verify-current --snapshot "$OUTPUT/source-snapshot.json"

# 官方完整插件集合静态合并，不通过库验证 entitlement 或动态回退规避限制。
RIME_PLUGINS='lua octagram predict' cmake -S "$SOURCE" -B "$BUILD" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=/usr/bin/clang -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
  -DCMAKE_OSX_ARCHITECTURES="$ARCH" -DCMAKE_OSX_DEPLOYMENT_TARGET=13.0 -DCMAKE_OSX_SYSROOT="$SDK" \
  -DCMAKE_PREFIX_PATH="$DEPS" -DCMAKE_IGNORE_PREFIX_PATH=/opt/homebrew \
  -DBOOST_ROOT="$BOOST" -DBoost_NO_SYSTEM_PATHS=ON -DBoost_NO_BOOST_CMAKE=ON \
  -DCMAKE_POLICY_DEFAULT_CMP0167=OLD -DCMAKE_POLICY_DEFAULT_CMP0144=NEW \
  -DX11Keysym="$DEPS/include" -DBUILD_SHARED_LIBS=OFF -DBUILD_STATIC=ON \
  -DBUILD_MERGED_PLUGINS=ON -DBUILD_TEST=OFF -DBUILD_TOOLS=OFF -DENABLE_EXTERNAL_PLUGINS=OFF \
  -DCMAKE_INSTALL_PREFIX="$OUTPUT"
cmake --build "$BUILD" --target rime-static -j "${STATIC_RIME_JOBS:-6}"
/usr/bin/python3 "$ROOT/fetch_sources.py" verify-current --snapshot "$OUTPUT/source-snapshot.json"

LIBRARIES=("$BUILD/lib/librime.a")
for dependency in glog leveldb marisa opencc yaml-cpp; do
  /usr/bin/lipo "$DEPS/lib/lib$dependency.a" -thin "$ARCH" -output "$OUTPUT/lib/lib$dependency.a"
  LIBRARIES+=("$OUTPUT/lib/lib$dependency.a")
done
/usr/bin/libtool -static -o "$OUTPUT/lib/libinputia_rime_static.a" "${LIBRARIES[@]}"
cp "$RELEASE/dist/include/"rime*.h "$OUTPUT/include/"
cp "$LICENSES/"* "$OUTPUT/licenses/"
cp "$ROOT/sources.lock.json" "$OUTPUT/sources.lock.json"

# 静态注册器采用 constructor；调用方需要 force_load 保留所有插件模块。
/usr/bin/python3 "$ROOT/artifact_binding.py" begin "$ARCH"
/usr/bin/clang++ -std=c++17 -arch "$ARCH" -isysroot "$SDK" -mmacosx-version-min=13.0 \
  -I "$OUTPUT/include" "$ROOT/probe/main.cpp" \
  "-Wl,-force_load,$OUTPUT/lib/libinputia_rime_static.a" -o "$OUTPUT/static-rime-probe"
/usr/bin/codesign --force --sign - --options runtime "$OUTPUT/static-rime-probe"
/usr/bin/python3 "$ROOT/artifact_binding.py" seal "$ARCH"
/usr/bin/python3 "$ROOT/verify_artifacts.py" "$ARCH" --publish
