#!/bin/zsh
set -eu

SOURCE_FILE="${1:?usage: verify-imk-event-route.sh <main.swift>}"

handle_count="$(/usr/bin/grep -c 'override func handle(_ event: NSEvent' "$SOURCE_FILE" || true)"
input_text_count="$(/usr/bin/grep -c 'override func inputText(' "$SOURCE_FILE" || true)"
command_count="$(/usr/bin/grep -c 'override func didCommand(' "$SOURCE_FILE" || true)"

if [[ "$handle_count" != "1" ]]; then
  echo "imkEventRoute=fail reason=expected-single-handle-route actual=$handle_count" >&2
  exit 1
fi

if [[ "$input_text_count" != "0" || "$command_count" != "0" ]]; then
  echo "imkEventRoute=fail reason=mixed-event-routes inputText=$input_text_count didCommand=$command_count" >&2
  exit 1
fi

echo "imkEventRoute=handle-all-events-only"
