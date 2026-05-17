#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ROOT="$ROOT/target/package/MeshMend.app"
CONTENTS="$APP_ROOT/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"
WORKERS="$RESOURCES/workers"

rm -rf "$APP_ROOT"
mkdir -p "$MACOS" "$WORKERS"

cp "$ROOT/target/release/meshmend" "$MACOS/meshmend"
chmod 755 "$MACOS/meshmend"

for worker in meshmend-cgal-worker meshmend-openvdb-worker; do
  cp "$ROOT/target/workers/cpp/$worker" "$WORKERS/$worker"
  chmod 755 "$WORKERS/$worker"
done

cat > "$CONTENTS/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>MeshMend</string>
  <key>CFBundleExecutable</key>
  <string>meshmend</string>
  <key>CFBundleIdentifier</key>
  <string>dev.meshmend.MeshMend</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>MeshMend</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

echo "$APP_ROOT"
