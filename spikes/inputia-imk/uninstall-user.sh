#!/usr/bin/env bash
set -euo pipefail

APP_NAME="InputiaIMKSpike.app"
TARGET_APP="$HOME/Library/Input Methods/$APP_NAME"
EXECUTABLE="$TARGET_APP/Contents/MacOS/InputiaIMKSpike"

if [[ -x "$EXECUTABLE" ]]; then
  "$EXECUTABLE" --disable-input-source || true
fi

rm -rf "$TARGET_APP"
echo "Removed $TARGET_APP"
