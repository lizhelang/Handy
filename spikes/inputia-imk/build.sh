#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="$ROOT_DIR/build"
APP_DIR="$BUILD_DIR/InputiaIMKSpike.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
SIGN_IDENTITY="${INPUTIA_CODESIGN_IDENTITY:--}"

rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

/usr/bin/swiftc \
  "$ROOT_DIR/main.swift" \
  -parse-as-library \
  -target "$(uname -m)-apple-macos13.0" \
  -module-name InputiaIMKSpike \
  -framework Cocoa \
  -framework InputMethodKit \
  -o "$MACOS_DIR/InputiaIMKSpike"

cp "$ROOT_DIR/Info.plist" "$CONTENTS_DIR/Info.plist"
cp "$ROOT_DIR/inputia.pdf" "$RESOURCES_DIR/inputia.pdf"
/usr/bin/plutil -lint "$CONTENTS_DIR/Info.plist"

if /usr/bin/codesign --force --sign "$SIGN_IDENTITY" "$APP_DIR" >/dev/null 2>&1; then
  /usr/bin/codesign --verify --deep --strict --verbose=2 "$APP_DIR"
else
  echo "warning: codesign failed with identity '$SIGN_IDENTITY'; build artifact still exists at $APP_DIR" >&2
fi

echo "$APP_DIR"
