#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Main}"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"

cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
  return 0
}

app_assessment() {
  if [[ -d "$1" ]]; then
    /usr/sbin/spctl --assess --type execute --verbose=4 "$1" 2>&1 || true
  fi
}

absolute_path() {
  /usr/bin/python3 - "$1" <<'PY'
import os
import sys

print(os.path.abspath(sys.argv[1]))
PY
}

tis_value() {
  local dump="$1"
  local source_id="$2"
  local key="$3"
  /usr/bin/awk -F= -v source_id="$source_id" -v key="$key" '
    $1 == "id" { active = ($2 == source_id) }
    active && $1 == key { print $2; exit }
  ' <<<"$dump"
}

tis_value_for_icon() {
  local dump="$1"
  local include="$2"
  local source_id="$3"
  local icon="$4"
  local key="$5"
  TIS_DUMP="$dump" /usr/bin/python3 - "$include" "$source_id" "$icon" "$key" <<'PY'
import os
import sys

include, source_id, icon, key = sys.argv[1:5]
active_include = None
current = None
sources = []
for line in os.environ.get("TIS_DUMP", "").splitlines():
    if line.startswith("includeAllInstalled="):
        if current:
            sources.append(current)
            current = None
        active_include = line.split("=", 1)[1]
        continue
    if active_include != include:
        continue
    if line.startswith("id="):
        if current:
            sources.append(current)
        current = {"id": line.split("=", 1)[1]}
        continue
    if current and "=" in line:
        name, value = line.split("=", 1)
        current[name] = value
if current:
    sources.append(current)

for source in sources:
    if source.get("id") == source_id and source.get("iconURL") == icon:
        print(source.get(key, ""))
        raise SystemExit(0)
for source in sources:
    if source.get("id") == source_id:
        print(source.get(key, ""))
        raise SystemExit(0)
PY
}

tis_count_for_icon() {
  local dump="$1"
  local include="$2"
  local source_id="$3"
  local icon="$4"
  TIS_DUMP="$dump" /usr/bin/python3 - "$include" "$source_id" "$icon" <<'PY'
import os
import sys

include, source_id, icon = sys.argv[1:4]
active_include = None
current = None
sources = []
for line in os.environ.get("TIS_DUMP", "").splitlines():
    if line.startswith("includeAllInstalled="):
        if current:
            sources.append(current)
            current = None
        active_include = line.split("=", 1)[1]
        continue
    if active_include != include:
        continue
    if line.startswith("id="):
        if current:
            sources.append(current)
        current = {"id": line.split("=", 1)[1]}
        continue
    if current and "=" in line:
        name, value = line.split("=", 1)
        current[name] = value
if current:
    sources.append(current)

print(sum(1 for source in sources if source.get("id") == source_id and source.get("iconURL") == icon))
PY
}

tis_matches() {
  local dump="$1"
  local include="$2"
  /usr/bin/awk -F= -v include="$include" '
    $1 == "includeAllInstalled" { active = ($2 == include) }
    active && $1 == "matches" { print $2; exit }
  ' <<<"$dump"
}

current_source_id() {
  if [[ -x "$TIS_TOOL" ]]; then
    "$TIS_TOOL" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return
  fi
  if [[ -x "$EXECUTABLE" ]]; then
    "$EXECUTABLE" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return
  fi
  echo ""
}

echo "app=$APP"
echo "buildApp=$BUILD_APP"
build_cdhash="$(cdhash "$BUILD_APP")"
app_cdhash="$(cdhash "$APP")"
echo "buildCDHash=$build_cdhash"
echo "appCDHash=$app_cdhash"
assessment="$(app_assessment "$APP")"
echo "appAssessment=$assessment"
if [[ "$assessment" == *": accepted"* ]]; then
  signature_accepted=true
else
  signature_accepted=false
fi
echo "appSignatureAccepted=$signature_accepted"
if [[ -n "$build_cdhash" && "$app_cdhash" == "$build_cdhash" ]]; then
  echo "appMatchesBuild=true"
else
  echo "appMatchesBuild=false"
fi

expected_icon="$(absolute_path "$APP/Contents/Resources/inputia.pdf")"
echo "expectedTISModeID=$TARGET_MODE_ID"
echo "expectedTISIcon=$expected_icon"

if [[ -x "$TIS_TOOL" ]]; then
  dump="$(INPUTIA_APP="$APP" "$TIS_TOOL" --dump 2>/dev/null || true)"
elif [[ -x "$EXECUTABLE" ]]; then
  echo "tisToolPresent=false path=$TIS_TOOL"
  echo "tisToolFallback=installed-host"
  dump="$(INPUTIA_APP="$APP" "$EXECUTABLE" --dump-input-source 2>/dev/null || true)"
else
  echo "tisToolPresent=false path=$TIS_TOOL"
  echo "tisReadiness=false reason=missing-tis-tool-and-host"
  exit 1
fi
enabled_matches="$(tis_matches "$dump" false)"
installed_matches="$(tis_matches "$dump" true)"
target_enabled_matches="$(tis_count_for_icon "$dump" false "$TARGET_MODE_ID" "$expected_icon")"
target_installed_matches="$(tis_count_for_icon "$dump" true "$TARGET_MODE_ID" "$expected_icon")"
hans_icon="$(tis_value_for_icon "$dump" true "$TARGET_MODE_ID" "$expected_icon" iconURL)"
hans_enabled="$(tis_value_for_icon "$dump" true "$TARGET_MODE_ID" "$expected_icon" enabled)"
hans_selected="$(tis_value_for_icon "$dump" true "$TARGET_MODE_ID" "$expected_icon" selected)"
hans_selectable="$(tis_value_for_icon "$dump" true "$TARGET_MODE_ID" "$expected_icon" selectable)"
if [[ "$hans_icon" == "$expected_icon" ]]; then
  icon_matches=true
else
  icon_matches=false
fi
current_id="$(current_source_id)"
if [[ "$current_id" == "$TARGET_MODE_ID" ]]; then
  current_matches=true
else
  current_matches=false
fi

echo "tis.enabledMatches=${enabled_matches:-unknown}"
echo "tis.installedMatches=${installed_matches:-unknown}"
echo "tis.targetEnabledMatches=${target_enabled_matches:-unknown}"
echo "tis.targetInstalledMatches=${target_installed_matches:-unknown}"
echo "tis.hansIconURL=${hans_icon:-unknown}"
echo "tis.hansIconMatchesApp=$icon_matches"
echo "tis.hansEnabled=${hans_enabled:-unknown}"
echo "tis.hansSelectable=${hans_selectable:-unknown}"
echo "tis.hansSelected=${hans_selected:-unknown}"
echo "tis.currentID=${current_id:-unknown}"
echo "tis.currentMatchesTarget=$current_matches"

if [[ "$signature_accepted" != "true" ]]; then
  echo "tis.readinessBlockReason=signature-rejected"
  echo "tis.requiredAction=sign-with-accepted-identity"
  echo "tisReadiness=false"
elif [[ "${target_enabled_matches:-0}" != "0" &&
  -n "${target_enabled_matches:-}" &&
  "$icon_matches" == "true" &&
  "${hans_enabled:-false}" == "true" ]]; then
  echo "tis.readinessBlockReason=none"
  echo "tisReadiness=true"
else
  if [[ "${target_installed_matches:-0}" == "0" || -z "${target_installed_matches:-}" ]]; then
    echo "tis.readinessBlockReason=target-source-not-installed"
  elif [[ "${target_enabled_matches:-0}" == "0" || -z "${target_enabled_matches:-}" ]]; then
    echo "tis.readinessBlockReason=missing-enabled-source"
  elif [[ "$icon_matches" != "true" ]]; then
    echo "tis.readinessBlockReason=icon-mismatch"
  elif [[ "${hans_enabled:-false}" != "true" ]]; then
    echo "tis.readinessBlockReason=hans-disabled"
  else
    echo "tis.readinessBlockReason=unknown"
  fi
  echo "tisReadiness=false"
fi
