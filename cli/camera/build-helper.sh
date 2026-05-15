#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="${1:-"$SCRIPT_DIR/../../build/camera"}"
mkdir -p "$OUT_DIR"

SDK="$(xcrun --sdk macosx --show-sdk-path)"
APP_DIR="$OUT_DIR/SimDeckCameraHelper.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
OUT="$MACOS_DIR/simdeck-camera-helper"
rm -rf "$APP_DIR"
rm -f "$OUT_DIR/simdeck-camera-helper"
mkdir -p "$MACOS_DIR"
cp "$SCRIPT_DIR/SimDeckCameraHelper-Info.plist" "$CONTENTS_DIR/Info.plist"

xcrun --sdk macosx clang \
  -fobjc-arc \
  -fmodules \
  -isysroot "$SDK" \
  -mmacosx-version-min=14.0 \
  -Wall \
  -Wextra \
  -framework Foundation \
  -framework AppKit \
  -framework AVFoundation \
  -framework CoreGraphics \
  -framework CoreImage \
  -framework CoreMedia \
  -framework CoreVideo \
  -framework ImageIO \
  -Wl,-sectcreate,__TEXT,__info_plist,"$SCRIPT_DIR/SimDeckCameraHelper-Info.plist" \
  "$SCRIPT_DIR/SimDeckCameraHelper.m" \
  -o "$OUT"

codesign --force --deep --sign - --entitlements "$SCRIPT_DIR/SimDeckCameraHelper.entitlements" "$APP_DIR" >/dev/null 2>&1 || true
chmod +x "$OUT"
echo "$OUT"
