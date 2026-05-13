#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
mkdir -p "$ROOT_DIR/build/bench"

X264_CFLAGS="$(pkg-config --cflags x264)"
X264_LIBS="$(pkg-config --libs x264)"

clang \
  -fobjc-arc \
  -fmodules \
  -I"$ROOT_DIR/cli" \
  $X264_CFLAGS \
  "$ROOT_DIR/scripts/bench/encoder-benchmark.m" \
  "$ROOT_DIR/cli/XCWH264Encoder.m" \
  -framework Foundation \
  -framework CoreVideo \
  -framework CoreMedia \
  -framework VideoToolbox \
  -framework QuartzCore \
  -framework Accelerate \
  $X264_LIBS \
  -o "$ROOT_DIR/build/bench/encoder-benchmark"

printf '%s\n' "$ROOT_DIR/build/bench/encoder-benchmark"
