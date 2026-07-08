#!/bin/bash
set -euo pipefail

TARGET_NAME="${INPUTIA_MENU_NAME:-Inputia}"

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

printf '%s\n' "$osascript_output" | tr '\r' '\n'

if ! /usr/bin/grep -q '^menuAgentFound=true$' <<<"$osascript_output"; then
  echo "menuReadiness=false"
  echo "menuReadinessBlockReason=menu-agent-unavailable"
  exit 0
fi
if ! /usr/bin/grep -q '^menuBarFound=true$' <<<"$osascript_output"; then
  echo "menuReadiness=false"
  echo "menuReadinessBlockReason=menu-bar-unavailable"
  exit 0
fi

inputia_count="$(
  /usr/bin/awk -F'|' -v target="$TARGET_NAME" '$1 ~ /^menuItem=/ && $2 == target { count++ } END { print count + 0 }' \
    <<<"$(printf '%s\n' "$osascript_output" | tr '\r' '\n')"
)"
selected_count="$(
  /usr/bin/awk -F'|' -v target="$TARGET_NAME" '$1 ~ /^menuItem=/ && $2 == target && $3 != "" { count++ } END { print count + 0 }' \
    <<<"$(printf '%s\n' "$osascript_output" | tr '\r' '\n')"
)"

echo "menuInputiaCount=$inputia_count"
echo "menuInputiaSelectedCount=$selected_count"
if [[ "$inputia_count" == "1" ]]; then
  echo "menuReadiness=true"
  echo "menuReadinessBlockReason=none"
else
  echo "menuReadiness=false"
  if [[ "$inputia_count" == "0" ]]; then
    echo "menuReadinessBlockReason=inputia-menu-item-missing"
  else
    echo "menuReadinessBlockReason=inputia-menu-item-duplicate"
  fi
fi
