#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT="$BUILD_DIR/simdeck"
OUTPUT_BIN="$BUILD_DIR/simdeck-bin"
MANIFEST_PATH="$ROOT_DIR/server/Cargo.toml"
SERVER_TARGET_DIR="$ROOT_DIR/server/target"
SERVER_BIN="$SERVER_TARGET_DIR/release/simdeck-server"

mkdir -p "$BUILD_DIR"

TMP_OUTPUT_BIN="$OUTPUT_BIN.tmp.$$"
trap 'rm -f "$TMP_OUTPUT_BIN"' EXIT

if [[ "${SIMDECK_BUILD_UNIVERSAL:-0}" == "1" ]]; then
  if ! command -v lipo >/dev/null 2>&1; then
    echo "SIMDECK_BUILD_UNIVERSAL=1 requires Apple lipo (Xcode Command Line Tools)." >&2
    exit 1
  fi

  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    if ! rustup target list --installed | grep -qx "$target"; then
      echo "Installing missing Rust target: $target"
      rustup target add "$target"
    fi
  done

  cargo build --release --manifest-path "$MANIFEST_PATH" --target aarch64-apple-darwin
  cargo build --release --manifest-path "$MANIFEST_PATH" --target x86_64-apple-darwin

  lipo -create \
    "$SERVER_TARGET_DIR/aarch64-apple-darwin/release/simdeck-server" \
    "$SERVER_TARGET_DIR/x86_64-apple-darwin/release/simdeck-server" \
    -output "$TMP_OUTPUT_BIN"

  echo "Universal binary archs:"
  lipo -info "$TMP_OUTPUT_BIN"
else
  cargo build --release --manifest-path "$MANIFEST_PATH"
  cp "$SERVER_BIN" "$TMP_OUTPUT_BIN"
fi

chmod +x "$TMP_OUTPUT_BIN"
mv -f "$TMP_OUTPUT_BIN" "$OUTPUT_BIN"
trap - EXIT

cat > "$OUTPUT" <<EOF
#!/usr/bin/env zsh
set -euo pipefail

SCRIPT_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
if [[ "\${1:-}" == "daemon" && "\${2:-}" == "run" ]]; then
  while true; do
    set +e
    "\$SCRIPT_DIR/$(basename "$OUTPUT_BIN")" "\$@"
    child_status=\$?
    set -e
    if [[ "\$child_status" == "75" ]]; then
      sleep 0.5
      continue
    fi
    exit "\$child_status"
  done
fi

exec "\$SCRIPT_DIR/$(basename "$OUTPUT_BIN")" "\$@"
EOF
chmod +x "$OUTPUT"

echo "Built $OUTPUT"
