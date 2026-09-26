#!/bin/zsh
set -euo pipefail

# 由已验证的 Python/MLX 环境构建 arm64 sidecar；模型目录独立传入，避免
# 在源码仓库或 Git 历史中复制 600MB 以上的权重。
ROOT_DIR="${0:A:h:h:h}"
PYTHON_BIN="${INPUTIA_LAYA_PYTHON:-/tmp/inputia-laya-venv/bin/python}"
MODEL_DIR="${INPUTIA_DECISION_MODEL_PATH:-}"
OUT_DIR="${INPUTIA_SIDECAR_OUTPUT:-$ROOT_DIR/src-tauri/resources/local-decision}"

[[ -x "$PYTHON_BIN" ]] || { print -u2 "missing Python runtime: $PYTHON_BIN"; exit 2; }
[[ -n "$MODEL_DIR" && -d "$MODEL_DIR" ]] || { print -u2 "set INPUTIA_DECISION_MODEL_PATH to a verified model directory"; exit 2; }

"$PYTHON_BIN" -m PyInstaller \
  native/local-decision/worker.py \
  --onedir --noconfirm --clean \
  --collect-all laya_mlx \
  --collect-all mlx \
  --hidden-import tokenizers \
  --distpath "$OUT_DIR" \
  --workpath "$OUT_DIR/.build" \
  --name inputia-decision-worker

MODEL_OUT="$OUT_DIR/laya-multilingual-mlx"
if [[ ! -d "$MODEL_OUT" ]]; then
  mkdir -p "$MODEL_OUT"
  cp -R "$MODEL_DIR"/. "$MODEL_OUT"/
fi

# Tauri's resource copier dereferences PyInstaller's libmlx.dylib symlink.
# When that happens MLX resolves the Metal library next to the expanded
# library path instead of the original mlx/lib directory. Keep an explicit
# copy at both locations so the frozen worker behaves identically in the
# source tree and inside the signed .app bundle.
METAL_LIB="$OUT_DIR/inputia-decision-worker/_internal/mlx/lib/mlx.metallib"
if [[ -f "$METAL_LIB" ]]; then
  mkdir -p "$OUT_DIR/inputia-decision-worker/_internal/lib"
  cp "$METAL_LIB" "$OUT_DIR/inputia-decision-worker/_internal/lib/mlx.metallib"
  cp "$METAL_LIB" "$OUT_DIR/inputia-decision-worker/_internal/mlx.metallib"
fi

print "sidecar=$OUT_DIR/inputia-decision-worker"
print "model=$MODEL_DIR"
