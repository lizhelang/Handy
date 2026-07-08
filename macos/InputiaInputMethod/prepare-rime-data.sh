#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="${INPUTIA_RIME_DATA_BUILD_DIR:-$ROOT_DIR/build/RimeData}"
DEFAULT_BUILD_DIR="$ROOT_DIR/build/RimeData"
SQUIRREL_SHARED="${INPUTIA_SQUIRREL_SHARED_DATA:-/Library/Input Methods/Squirrel.app/Contents/SharedSupport}"
INSTALLED_INPUTIA_RIME_DATA="/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData"
PREVIOUS_BUILD_DIR=""

if [[ -d "$BUILD_DIR" ]]; then
  PREVIOUS_BUILD_DIR="${BUILD_DIR}.previous.$$"
  /bin/rm -rf "$PREVIOUS_BUILD_DIR"
  COPYFILE_DISABLE=1 /usr/bin/ditto "$BUILD_DIR" "$PREVIOUS_BUILD_DIR"
fi

cleanup_previous_build_dir() {
  if [[ -n "$PREVIOUS_BUILD_DIR" ]]; then
    /bin/rm -rf "$PREVIOUS_BUILD_DIR"
  fi
}
trap cleanup_previous_build_dir EXIT

/bin/rm -rf "$BUILD_DIR"
/bin/mkdir -p "$BUILD_DIR"

copy_or_download() {
  local url="$1"
  local output="$2"

  for fallback_dir in "$PREVIOUS_BUILD_DIR" "$INSTALLED_INPUTIA_RIME_DATA" "$ROOT_DIR/Resources/RimeData" "$DEFAULT_BUILD_DIR"; do
    if [[ -n "$fallback_dir" && -f "$fallback_dir/$output" ]]; then
      echo "reuseRimeData=$output from=$fallback_dir"
      /bin/mkdir -p "$BUILD_DIR/$(/usr/bin/dirname "$output")"
      /bin/cp "$fallback_dir/$output" "$BUILD_DIR/$output"
      return
    fi
  done

  echo "fetchRimeData=$output"
  /bin/mkdir -p "$BUILD_DIR/$(/usr/bin/dirname "$output")"
  /usr/bin/curl --connect-timeout 10 --max-time 45 --retry 2 --retry-delay 1 -fsSL \
    "$url" -o "$BUILD_DIR/$output"
}

bootstrap_official_rime_data() {
  echo "squirrelSharedMissing=$SQUIRREL_SHARED"
  echo "bootstrapRimeData=official-rime-packages"

  local prelude="https://raw.githubusercontent.com/rime/rime-prelude/master"
  local luna="https://raw.githubusercontent.com/rime/rime-luna-pinyin/master"
  local essay="https://raw.githubusercontent.com/rime/rime-essay/master"
  local stroke="https://raw.githubusercontent.com/rime/rime-stroke/master"
  local terra="https://raw.githubusercontent.com/rime/rime-terra-pinyin/master"

  copy_or_download "$prelude/default.yaml" default.yaml
  copy_or_download "$prelude/key_bindings.yaml" key_bindings.yaml
  copy_or_download "$prelude/punctuation.yaml" punctuation.yaml
  copy_or_download "$prelude/symbols.yaml" symbols.yaml
  copy_or_download "$essay/essay.txt" essay.txt
  copy_or_download "$luna/pinyin.yaml" pinyin.yaml
  copy_or_download "$luna/luna_pinyin.schema.yaml" luna_pinyin.schema.yaml
  copy_or_download "$luna/luna_pinyin_simp.schema.yaml" luna_pinyin_simp.schema.yaml
  copy_or_download "$luna/luna_pinyin.dict.yaml" luna_pinyin.dict.yaml
  copy_or_download "$stroke/stroke.schema.yaml" stroke.schema.yaml
  copy_or_download "$stroke/stroke.dict.yaml" stroke.dict.yaml
  copy_or_download "$terra/terra_pinyin.schema.yaml" terra_pinyin.schema.yaml
  copy_or_download "$terra/terra_pinyin.dict.yaml" terra_pinyin.dict.yaml
}

if [[ -d "$SQUIRREL_SHARED" ]]; then
  echo "copySquirrelShared=$SQUIRREL_SHARED"
  COPYFILE_DISABLE=1 /usr/bin/ditto "$SQUIRREL_SHARED" "$BUILD_DIR"
else
  bootstrap_official_rime_data
fi

download_schema() {
  local url="$1"
  local output="$2"

  for fallback_dir in "$PREVIOUS_BUILD_DIR" "$INSTALLED_INPUTIA_RIME_DATA" "$ROOT_DIR/Resources/RimeData" "$DEFAULT_BUILD_DIR"; do
    if [[ -n "$fallback_dir" && -f "$fallback_dir/$output" ]]; then
      echo "reuseRimeSchema=$output from=$fallback_dir"
      /bin/cp "$fallback_dir/$output" "$BUILD_DIR/$output"
      return
    fi
  done

  echo "fetchRimeSchema=$output"
  if /usr/bin/curl --connect-timeout 10 --max-time 45 --retry 2 --retry-delay 1 -fsSL "$url" -o "$BUILD_DIR/$output"; then
    return
  fi

  echo "missing Rime schema and no local fallback: $output" >&2
  return 1
}

download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/AUTHORS rime-double-pinyin.AUTHORS
download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/LICENSE rime-double-pinyin.LICENSE
download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/double_pinyin.schema.yaml double_pinyin.schema.yaml
download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/double_pinyin_abc.schema.yaml double_pinyin_abc.schema.yaml
download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/double_pinyin_flypy.schema.yaml double_pinyin_flypy.schema.yaml
download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/double_pinyin_mspy.schema.yaml double_pinyin_mspy.schema.yaml
download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/double_pinyin_pyjj.schema.yaml double_pinyin_pyjj.schema.yaml
download_schema https://raw.githubusercontent.com/rime/rime-double-pinyin/master/double_pinyin_st.schema.yaml double_pinyin_st.schema.yaml
download_schema https://raw.githubusercontent.com/baopaau/rime-guobiao-quick/main/README.md rime-guobiao-quick.README.md
download_schema https://raw.githubusercontent.com/baopaau/rime-guobiao-quick/main/guobiao_bispell.schema.yaml guobiao_bispell.schema.yaml

# The upstream Guobiao schema enables an optional emoji OpenCC filter, but the
# bundled Squirrel shared data used by the MVP does not include emoji.json.
# Strip that filter during packaging so Guobiao input does not emit runtime
# OpenCC errors while keeping the core double-pinyin schema intact.
/usr/bin/awk '
  /^  - name: emoji_suggestion$/ {
    getline
    getline
    next
  }
  /^    - simplifier@emoji_suggestion$/ {
    next
  }
  /^emoji_suggestion:$/ {
    getline
    getline
    getline
    next
  }
  {
    print
  }
' "$BUILD_DIR/guobiao_bispell.schema.yaml" >"$BUILD_DIR/guobiao_bispell.schema.yaml.tmp"
/bin/mv "$BUILD_DIR/guobiao_bispell.schema.yaml.tmp" "$BUILD_DIR/guobiao_bispell.schema.yaml"

/usr/bin/awk '
  BEGIN {
    skipping = 0
  }
  /^schema_list:/ {
    print "schema_list:"
    print "  - schema: luna_pinyin_simp"
    print "  - schema: double_pinyin"
    print "  - schema: double_pinyin_flypy"
    print "  - schema: double_pinyin_sogou"
    print "  - schema: guobiao_bispell"
    print "  - schema: double_pinyin_mspy"
    print "  - schema: double_pinyin_abc"
    print "  - schema: double_pinyin_pyjj"
    print "  - schema: double_pinyin_st"
    print "  - schema: stroke"
    print "  - schema: terra_pinyin"
    skipping = 1
    next
  }
  skipping && /^[^[:space:]]/ {
    skipping = 0
  }
  !skipping {
    print
  }
' "$BUILD_DIR/default.yaml" >"$BUILD_DIR/default.yaml.tmp"
/bin/mv "$BUILD_DIR/default.yaml.tmp" "$BUILD_DIR/default.yaml"

/bin/cat >"$BUILD_DIR/double_pinyin_sogou.schema.yaml" <<'YAML'
# Rime schema
# encoding: utf-8
#
# Inputia local MVP schema.
# 搜狗双拼键位映射取自 iDvel/rime-ice 的 double_pinyin_sogou.schema.yaml；
# 这里使用朙月拼音词典，避免引入 rime-ice 的 Lua/词库依赖。

schema:
  schema_id: double_pinyin_sogou
  name: 搜狗双拼
  version: "0.1-inputia"
  author:
    - Inputia contributors
    - Sogou layout via iDvel/rime-ice
  description: |
    朙月拼音＋搜狗双拼方案。
  dependencies:
    - stroke

switches:
  - name: ascii_mode
    reset: 0
    states: [ 中文, 西文 ]
  - name: full_shape
    states: [ 半角, 全角 ]
  - name: simplification
    states: [ 漢字, 汉字 ]
  - name: ascii_punct
    states: [ 。，, ．， ]

engine:
  processors:
    - ascii_composer
    - recognizer
    - key_binder
    - speller
    - punctuator
    - selector
    - navigator
    - express_editor
  segmentors:
    - ascii_segmentor
    - matcher
    - abc_segmentor
    - punct_segmentor
    - fallback_segmentor
  translators:
    - punct_translator
    - reverse_lookup_translator
    - script_translator
  filters:
    - simplifier
    - uniquifier

speller:
  alphabet: zyxwvutsrqponmlkjihgfedcba;
  delimiter: " '"
  algebra:
    - erase/^xx$/
    - derive/^([jqxy])u$/$1v/
    - derive/^([aoe].*)$/o$1/
    - xform/^([ae])(.*)$/$1$1$2/
    - xform/iu$/Q/
    - xform/[iu]a$/W/
    - xform/er$|[uv]an$/R/
    - xform/[uv]e$/T/
    - xform/v$|uai$/Y/
    - xform/^sh/U/
    - xform/^ch/I/
    - xform/^zh/V/
    - xform/uo$/O/
    - xform/[uv]n$/P/
    - xform/(.)i?ong$/$1S/
    - xform/[iu]ang$/D/
    - xform/(.)en$/$1F/
    - xform/(.)eng$/$1G/
    - xform/(.)ang$/$1H/
    - xform/ian$/M/
    - xform/(.)an$/$1J/
    - xform/iao$/C/
    - xform/(.)ao$/$1K/
    - xform/(.)ai$/$1L/
    - xform/(.)ei$/$1Z/
    - xform/ie$/X/
    - xform/ui$/V/
    - xform/(.)ou$/$1B/
    - xform/in$/N/
    - xform/ing$/;/
    - xlit/QWRTYUIOPSDFGHMJCKLZXVBN/qwrtyuiopsdfghmjcklzxvbn/

translator:
  dictionary: luna_pinyin
  prism: double_pinyin_sogou
  preedit_format:
    - xform/([aoe])(\w)/0$2/
    - xform/([bpmnljqxy])n/$1in/
    - xform/(\w)g/$1eng/
    - xform/(\w)q/$1iu/
    - xform/([gkhvuirzcs])w/$1ua/
    - xform/(\w)w/$1ia/
    - xform/([dtnlgkhjqxyvuirzcs])r/$1uan/
    - xform/0r/er/
    - xform/([dtgkhvuirzcs])v/$1ui/
    - xform/(\w)v/$1ve/
    - xform/(\w)t/$1ve/
    - xform/([gkhvuirzcs])y/$1uai/
    - xform/(\w)y/$1v/
    - xform/([dtnlgkhvuirzcs])o/$1uo/
    - xform/(\w)p/$1un/
    - xform/([jqx])s/$1iong/
    - xform/(\w)s/$1ong/
    - xform/([jqxnlb])d/$1iang/
    - xform/(\w)d/$1uang/
    - xform/(\w)f/$1en/
    - xform/(\w)h/$1ang/
    - xform/(\w)j/$1an/
    - xform/(\w)k/$1ao/
    - xform/(\w)l/$1ai/
    - xform/(\w)z/$1ei/
    - xform/(\w)x/$1ie/
    - xform/(\w)c/$1iao/
    - xform/(\w)b/$1ou/
    - xform/(\w)m/$1ian/
    - xform/(\w);/$1ing/
    - xform/0(\w)/$1/
    - "xform/(^|[ '])v/$1zh/"
    - "xform/(^|[ '])i/$1ch/"
    - "xform/(^|[ '])u/$1sh/"
    - xform/([jqxy])v/$1u/
    - xform/([nl])v/$1ü/
    - xform/ü/v/

reverse_lookup:
  dictionary: stroke
  enable_completion: true
  prefix: "`"
  suffix: "'"
  tips: 〔筆畫〕
  preedit_format:
    - xlit/hspnz/一丨丿丶乙/
  comment_format:
    - xform/([nl])v/$1ü/

punctuator:
  import_preset: default

key_binder:
  import_preset: default

recognizer:
  import_preset: default
  patterns:
    reverse_lookup: "`[a-z]*'?$"
YAML

echo "rimeDataDir=$BUILD_DIR"
