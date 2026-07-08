#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
USER_DEFAULT_APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
DEFAULT_APP="/Library/Input Methods/InputiaInputMethod.app"
if [[ -d "$USER_DEFAULT_APP" ]]; then
  DEFAULT_APP="$USER_DEFAULT_APP"
fi
APP="${1:-$DEFAULT_APP}"
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

user_directory_ready() {
  /usr/bin/python3 <<'PY'
import os
import pwd

try:
    pwd.getpwuid(os.getuid())
except KeyError:
    print("false")
else:
    print("true")
PY
}

hitoolbox_defaults_readable() {
  if /usr/bin/defaults read com.apple.HIToolbox >/dev/null 2>&1; then
    echo true
  else
    echo false
  fi
}

hitoolbox_key_contains_inputia() {
  local key="$1"
  local output
  if ! output="$(/usr/bin/defaults read com.apple.HIToolbox "$key" 2>/dev/null)"; then
    echo unknown
    return
  fi
  if /usr/bin/grep -q 'com\.inputia\.inputmethod\.Inputia' <<<"$output"; then
    echo true
  else
    echo false
  fi
}

menu_source_block_reason() {
  local reason="$1"
  case "$reason" in
  none)
    echo none
    ;;
  inputia-menu-item-missing)
    echo menu-source-missing
    ;;
  inputia-menu-item-duplicate)
    echo menu-source-duplicate
    ;;
  menu-agent-unavailable)
    echo text-input-menu-agent-unavailable
    ;;
  menu-bar-unavailable)
    echo text-input-menu-bar-unavailable
    ;;
  skipped)
    echo menu-readiness-skipped
    ;;
  missing-menu-readiness-script)
    echo missing-menu-readiness-script
    ;;
  *)
    echo menu-readiness-unknown
    ;;
  esac
}

menu_presentation_block_reason() {
  local reason="$1"
  case "$reason" in
  inputia-menu-item-missing)
    echo text-input-menu-agent-not-presenting-source
    ;;
  inputia-menu-item-duplicate)
    echo text-input-menu-agent-presenting-duplicate-source
    ;;
  menu-agent-unavailable)
    echo text-input-menu-agent-unavailable
    ;;
  menu-bar-unavailable)
    echo text-input-menu-bar-unavailable
    ;;
  skipped)
    echo menu-readiness-skipped
    ;;
  missing-menu-readiness-script)
    echo missing-menu-readiness-script
    ;;
  none)
    echo none
    ;;
  *)
    echo menu-readiness-unknown
    ;;
  esac
}

echo "app=$APP"
echo "buildApp=$BUILD_APP"
if [[ -d "$APP" ]]; then
  app_exists=true
else
  app_exists=false
fi
echo "appExists=$app_exists"
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
user_dir_ready="$(user_directory_ready)"
hitoolbox_defaults_ok="$(hitoolbox_defaults_readable)"
legacy_hitoolbox_enabled="$(hitoolbox_key_contains_inputia AppleEnabledInputSources)"
legacy_hitoolbox_selected="$(hitoolbox_key_contains_inputia AppleSelectedInputSources)"
if [[ "$legacy_hitoolbox_enabled" == "true" && "$current_matches" != "true" ]]; then
  stale_hitoolbox_enabled=true
else
  stale_hitoolbox_enabled=false
fi
menu_readiness=unknown
menu_block_reason=unknown
menu_inputia_count=unknown
menu_selected_count=unknown
if [[ "${INPUTIA_TIS_SKIP_MENU_READINESS:-0}" == "1" ]]; then
  menu_readiness=unknown
  menu_block_reason=skipped
elif [[ -x "$ROOT_DIR/menu-readiness.sh" ]]; then
  menu_output="$("$ROOT_DIR/menu-readiness.sh" 2>&1 || true)"
  menu_readiness="$(/usr/bin/awk -F= '$1 == "menuReadiness" { print $2; exit }' <<<"$menu_output")"
  menu_block_reason="$(/usr/bin/awk -F= '$1 == "menuReadinessBlockReason" { print $2; exit }' <<<"$menu_output")"
  menu_inputia_count="$(/usr/bin/awk -F= '$1 == "menuInputiaCount" { print $2; exit }' <<<"$menu_output")"
  menu_selected_count="$(/usr/bin/awk -F= '$1 == "menuInputiaSelectedCount" { print $2; exit }' <<<"$menu_output")"
  menu_readiness="${menu_readiness:-unknown}"
  menu_block_reason="${menu_block_reason:-unknown}"
  menu_inputia_count="${menu_inputia_count:-unknown}"
  menu_selected_count="${menu_selected_count:-unknown}"
else
  menu_readiness=false
  menu_block_reason=missing-menu-readiness-script
fi
menu_source_reason="$(menu_source_block_reason "$menu_block_reason")"
menu_presentation_reason="$(menu_presentation_block_reason "$menu_block_reason")"

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
echo "tis.userDirectoryReady=$user_dir_ready"
echo "tis.hitoolboxDefaultsReadable=$hitoolbox_defaults_ok"
echo "tis.legacyHIToolboxInputiaEnabled=$legacy_hitoolbox_enabled"
echo "tis.legacyHIToolboxInputiaSelected=$legacy_hitoolbox_selected"
echo "tis.staleHIToolboxEnabledStateSuspected=$stale_hitoolbox_enabled"
echo "tis.menuReadiness=$menu_readiness"
echo "tis.menuInputiaCount=$menu_inputia_count"
echo "tis.menuInputiaSelectedCount=$menu_selected_count"
echo "tis.menuBlockReason=$menu_block_reason"
echo "tis.menuSourceBlockReason=$menu_source_reason"
echo "tis.menuPresentationBlockReason=$menu_presentation_reason"

if [[ "$app_exists" != "true" ]]; then
  echo "tis.readinessBlockReason=app-missing"
  echo "tis.requiredAction=install-inputia-app"
  if [[ "$user_dir_ready" != "true" || "$hitoolbox_defaults_ok" != "true" ]]; then
    echo "tis.environmentRequiredAction=repair-current-user-directory-service"
  fi
  echo "tisReadiness=false"
elif [[ "$signature_accepted" != "true" ]]; then
  echo "tis.readinessBlockReason=signature-rejected"
  echo "tis.requiredAction=sign-with-accepted-identity"
  echo "tisReadiness=false"
else
  if [[ "${target_installed_matches:-0}" == "0" || -z "${target_installed_matches:-}" ]]; then
    echo "tis.readinessBlockReason=target-source-not-installed"
  elif [[ "${target_enabled_matches:-0}" == "0" || -z "${target_enabled_matches:-}" ]]; then
    echo "tis.readinessBlockReason=missing-enabled-source"
    if [[ "$user_dir_ready" != "true" || "$hitoolbox_defaults_ok" != "true" ]]; then
      echo "tis.requiredAction=repair-current-user-directory-service"
    fi
  elif [[ "$icon_matches" != "true" ]]; then
    echo "tis.readinessBlockReason=icon-mismatch"
  elif [[ "${hans_enabled:-false}" != "true" ]]; then
    echo "tis.readinessBlockReason=hans-disabled"
  elif [[ "$menu_source_reason" != "none" ]]; then
    echo "tis.readinessBlockReason=$menu_source_reason"
    echo "tis.requiredAction=fix-input-source-metadata-or-registration-cache"
    if [[ "$menu_presentation_reason" != "none" ]]; then
      echo "tis.presentationRequiredAction=$menu_presentation_reason"
    fi
  elif [[ "$current_matches" != "true" ]]; then
    echo "tis.readinessBlockReason=target-not-selected"
    echo "tis.requiredAction=select-visible-inputia-source"
  else
    echo "tis.readinessBlockReason=none"
    echo "tisReadiness=true"
    exit 0
  fi
  echo "tisReadiness=false"
fi
