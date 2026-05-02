#!/bin/sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
mkdir -p "$ROOT_DIR/build/bench"

clang \
  -fobjc-arc \
  -fmodules \
  -I"$ROOT_DIR/cli" \
  "$ROOT_DIR/scripts/bench/encoder-benchmark.m" \
  "$ROOT_DIR/cli/XCWH264Encoder.m" \
  -framework Foundation \
  -framework CoreVideo \
  -framework CoreMedia \
  -framework VideoToolbox \
  -framework QuartzCore \
  -framework Accelerate \
  -o "$ROOT_DIR/build/bench/encoder-benchmark"

printf '%s\n' "$ROOT_DIR/build/bench/encoder-benchmark"
