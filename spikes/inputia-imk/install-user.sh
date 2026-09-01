#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="InputiaIMKSpike.app"
SOURCE_APP="$ROOT_DIR/build/$APP_NAME"
INSTALL_DIR="$HOME/Library/Input Methods"
TARGET_APP="$INSTALL_DIR/$APP_NAME"
EXECUTABLE="$TARGET_APP/Contents/MacOS/InputiaIMKSpike"

"$ROOT_DIR/build.sh" >/dev/null

mkdir -p "$INSTALL_DIR"
rm -rf "$TARGET_APP"
/usr/bin/ditto "$SOURCE_APP" "$TARGET_APP"

"$EXECUTABLE" --register-input-source
"$EXECUTABLE" --enable-input-source
"$EXECUTABLE" --list-input-source

echo "$TARGET_APP"
