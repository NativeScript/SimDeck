#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="${1:-"$SCRIPT_DIR/../../build/camera"}"
mkdir -p "$OUT_DIR"

SDK="$(xcrun --sdk iphonesimulator --show-sdk-path)"
OUT="$OUT_DIR/libSimDeckCameraInjector.dylib"
ARCHES=()
HOST_ARCH="$(uname -m)"
if [[ "$HOST_ARCH" == "arm64" ]]; then
  ARCHES=(-arch arm64)
else
  ARCHES=(-arch x86_64)
fi

xcrun --sdk iphonesimulator clang \
  "${ARCHES[@]}" \
  -dynamiclib \
  -fmodules \
  -isysroot "$SDK" \
  -mios-simulator-version-min=15.0 \
  -Wall \
  -Wextra \
  -framework Foundation \
  -framework AVFoundation \
  -framework CoreGraphics \
  -framework CoreMedia \
  -framework CoreVideo \
  -framework QuartzCore \
  -framework UIKit \
  -install_name "@rpath/libSimDeckCameraInjector.dylib" \
  "$SCRIPT_DIR/SimDeckCameraInjector.m" \
  -o "$OUT"

codesign --force --sign - "$OUT" >/dev/null 2>&1 || true
echo "$OUT"
