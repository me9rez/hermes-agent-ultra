#!/bin/sh
set -e

PLIST_SRC="/Library/Application Support/Terra/app.terra.http.plist"
PLIST_DST="$HOME/Library/LaunchAgents/app.terra.http.plist"
UID_NUM="$(id -u)"

mkdir -p "$HOME/Library/LaunchAgents"
cp "$PLIST_SRC" "$PLIST_DST"
launchctl bootout "gui/$UID_NUM" "$PLIST_DST" 2>/dev/null || true
launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"
launchctl enable "gui/$UID_NUM/app.terra.http"
