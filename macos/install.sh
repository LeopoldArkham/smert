#!/bin/sh
# Builds the wallpaper agent and installs it as a launchd agent that starts at
# login and is restarted if it exits. Run it again after changing the source.
#
#   ./install.sh             build, install and start
#   ./install.sh uninstall   stop and remove the agent (the wallpaper stays)
set -eu

LABEL=com.leopoldarkham.smert.wallpaper
HERE=$(cd "$(dirname "$0")" && pwd)
SUPPORT="$HOME/Library/Application Support/smert-wallpaper"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$(id -u)"

launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true

if [ "${1:-}" = uninstall ]; then
    rm -f "$PLIST"
    rm -rf "$SUPPORT"
    echo "Uninstalled. Pick another wallpaper in System Settings > Wallpaper."
    exit 0
fi

mkdir -p "$SUPPORT/fonts"
swiftc -O -o "$SUPPORT/SmertWallpaper" "$HERE/SmertWallpaper.swift"
cp "$HERE/../android/app/src/main/res/font/baskervville.ttf" \
   "$HERE/../android/app/src/main/res/font/baskervville_italic.ttf" "$SUPPORT/fonts/"
# Images drawn with the old build or fonts would otherwise be reused.
rm -rf "$SUPPORT/wallpapers"

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>$SUPPORT/SmertWallpaper</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ProcessType</key>
    <string>Background</string>
    <key>StandardErrorPath</key>
    <string>$SUPPORT/agent.log</string>
</dict>
</plist>
EOF

launchctl bootstrap "$DOMAIN" "$PLIST"
echo "Installed; log at $SUPPORT/agent.log"
