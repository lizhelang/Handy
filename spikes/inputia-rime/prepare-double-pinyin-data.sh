#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCHEMA_REPO="${INPUTIA_RIME_DOUBLE_PINYIN_REPO:-/tmp/inputia-rime-double-pinyin}"
SHARED_DIR="${INPUTIA_RIME_DOUBLE_PINYIN_SHARED_DIR:-/tmp/inputia-rime-shared-double-pinyin}"
USER_DIR="${INPUTIA_RIME_DOUBLE_PINYIN_USER_DIR:-/tmp/inputia-rime-user-double-pinyin}"
SQUIRREL_APP="${SQUIRREL_APP:-/Library/Input Methods/Squirrel.app}"
SQUIRREL_SHARED="$SQUIRREL_APP/Contents/SharedSupport"
RIME_DEPLOYER="$SQUIRREL_APP/Contents/MacOS/rime_deployer"
STAGING_DIR="$USER_DIR/build"
SCHEMA_ID="${1:-double_pinyin_flypy}"

if [ ! -d "$SCHEMA_REPO/.git" ]; then
  git clone --depth 1 https://github.com/rime/rime-double-pinyin.git "$SCHEMA_REPO"
fi

if [ ! -d "$SQUIRREL_SHARED" ]; then
  echo "missing Squirrel shared data: $SQUIRREL_SHARED" >&2
  exit 1
fi

if [ ! -x "$RIME_DEPLOYER" ]; then
  echo "missing rime_deployer: $RIME_DEPLOYER" >&2
  exit 1
fi

rm -rf "$SHARED_DIR" "$USER_DIR"
mkdir -p "$SHARED_DIR" "$USER_DIR" "$STAGING_DIR"

/usr/bin/ditto "$SQUIRREL_SHARED" "$SHARED_DIR"
cp "$SCHEMA_REPO"/*.schema.yaml "$SHARED_DIR/"

(
  cd "$USER_DIR"
  "$RIME_DEPLOYER" --add-schema "$SCHEMA_ID" >/dev/null
  "$RIME_DEPLOYER" --set-active-schema "$SCHEMA_ID" >/dev/null
)

"$RIME_DEPLOYER" --build "$USER_DIR" "$SHARED_DIR" "$STAGING_DIR" >/dev/null
"$ROOT_DIR/build.sh" >/dev/null

echo "schema=$SCHEMA_ID"
echo "shared=$SHARED_DIR"
echo "user=$USER_DIR"
echo "probe=$ROOT_DIR/build/rime_probe"
