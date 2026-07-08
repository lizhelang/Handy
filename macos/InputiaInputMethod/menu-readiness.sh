#!/bin/bash
set -euo pipefail

TARGET_NAME="${INPUTIA_MENU_NAME:-}"
CACHE_FILE="${INPUTIA_MENU_READINESS_CACHE_FILE:-}"

if [[ -n "$CACHE_FILE" && -s "$CACHE_FILE" && "${INPUTIA_MENU_READINESS_DISABLE_CACHE:-0}" != "1" ]]; then
  echo "menuReadinessCache=hit path=$CACHE_FILE"
  /bin/cat "$CACHE_FILE"
  exit 0
fi

osascript_output="$(
  /usr/bin/osascript <<'APPLESCRIPT' 2>&1 || true
tell application "System Events"
  if not (exists process "TextInputMenuAgent") then
    return "menuAgentFound=false"
  end if
  tell process "TextInputMenuAgent"
    if (count of menu bars) < 2 then
      return "menuAgentFound=true" & linefeed & "menuBarFound=false"
    end if
    tell menu bar item 1 of menu bar 2
      perform action "AXPress"
      delay 0.3
      set out to {"menuAgentFound=true", "menuBarFound=true"}
      set itemCount to count of menu items of menu 1
      set end of out to "menuItemCount=" & itemCount
      repeat with i from 1 to itemCount
        set itemName to ""
        set markValue to ""
        try
          set itemName to name of menu item i of menu 1
        end try
        try
          set markValue to value of attribute "AXMenuItemMarkChar" of menu item i of menu 1
        end try
        if itemName is missing value then set itemName to "<separator>"
        if markValue is missing value then set markValue to ""
        set end of out to "menuItem=" & i & "|" & itemName & "|" & markValue
      end repeat
      key code 53
      set AppleScript's text item delimiters to linefeed
      set renderedOutput to out as string
      set AppleScript's text item delimiters to ""
      return renderedOutput
    end tell
  end tell
end tell
APPLESCRIPT
)"

normalized_output="$(printf '%s\n' "$osascript_output" | tr '\r' '\n')"
final_output="$normalized_output"

append_line() {
  final_output="${final_output}"$'\n'"$1"
}

finish() {
  if [[ -n "$CACHE_FILE" ]]; then
    /bin/mkdir -p "$(/usr/bin/dirname "$CACHE_FILE")"
    tmp_cache="${CACHE_FILE}.$$"
    printf '%s\n' "$final_output" >"$tmp_cache"
    /bin/mv "$tmp_cache" "$CACHE_FILE"
  fi
  printf '%s\n' "$final_output"
}

if ! /usr/bin/grep -q '^menuAgentFound=true$' <<<"$normalized_output"; then
  append_line "menuReadiness=false"
  append_line "menuReadinessBlockReason=menu-agent-unavailable"
  finish
  exit 0
fi
if ! /usr/bin/grep -q '^menuBarFound=true$' <<<"$normalized_output"; then
  append_line "menuReadiness=false"
  append_line "menuReadinessBlockReason=menu-bar-unavailable"
  finish
  exit 0
fi

inputia_count="$(
  /usr/bin/awk -F'|' -v target="$TARGET_NAME" '
    function is_inputia_source(name) {
      return name == "Inputia" ||
        name == "Inputia 简体" ||
        name == "Inputia 簡體" ||
        name == "Inputia 繁体" ||
        name == "Inputia 繁體" ||
        name == "Inputia Simplified" ||
        name == "Inputia Traditional"
    }
    BEGIN { source_block = 1 }
    $1 ~ /^menuItem=/ && $2 == "<separator>" {
      source_block = 0
      next
    }
    $1 ~ /^menuItem=/ {
      if (target != "" && $2 == target) {
        count++
      } else if (target == "" && source_block && is_inputia_source($2)) {
        count++
      }
    }
    END { print count + 0 }
  ' \
    <<<"$normalized_output"
)"
selected_count="$(
  /usr/bin/awk -F'|' -v target="$TARGET_NAME" '
    function is_inputia_source(name) {
      return name == "Inputia" ||
        name == "Inputia 简体" ||
        name == "Inputia 簡體" ||
        name == "Inputia 繁体" ||
        name == "Inputia 繁體" ||
        name == "Inputia Simplified" ||
        name == "Inputia Traditional"
    }
    BEGIN { source_block = 1 }
    $1 ~ /^menuItem=/ && $2 == "<separator>" {
      source_block = 0
      next
    }
    $1 ~ /^menuItem=/ && $3 != "" {
      if (target != "" && $2 == target) {
        count++
      } else if (target == "" && source_block && is_inputia_source($2)) {
        count++
      }
    }
    END { print count + 0 }
  ' \
    <<<"$normalized_output"
)"
selected_separator_count="$(
  /usr/bin/awk -F'|' '$1 ~ /^menuItem=/ && $2 == "<separator>" && $3 != "" { count++ } END { print count + 0 }' \
    <<<"$normalized_output"
)"

append_line "menuInputiaCount=$inputia_count"
append_line "menuInputiaSelectedCount=$selected_count"
append_line "menuSelectedSeparatorCount=$selected_separator_count"
if [[ "$selected_separator_count" != "0" ]]; then
  append_line "menuReadiness=false"
  append_line "menuReadinessBlockReason=inputia-menu-selected-separator"
elif [[ "$inputia_count" -ge 1 ]]; then
  append_line "menuReadiness=true"
  append_line "menuReadinessBlockReason=none"
else
  append_line "menuReadiness=false"
  append_line "menuReadinessBlockReason=inputia-menu-item-missing"
fi
finish
