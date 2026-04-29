#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT="$BUILD_DIR/simdeck"
OUTPUT_BIN="$BUILD_DIR/simdeck-bin"
MANIFEST_PATH="$ROOT_DIR/server/Cargo.toml"
SERVER_TARGET_DIR="$ROOT_DIR/server/target"

# SimDeck depends on AArch64 inline asm in cli/*.m and Apple Silicon-only
# private CoreSimulator behavior, so the binary is arm64-only by design.
# Optionally pin the build to an explicit Rust target triple via
# SIMDECK_BUILD_TARGET (the release workflow uses aarch64-apple-darwin); when
# unset we use the host triple so local dev on Apple Silicon stays fast.

mkdir -p "$BUILD_DIR"

TMP_OUTPUT_BIN="$OUTPUT_BIN.tmp.$$"
trap 'rm -f "$TMP_OUTPUT_BIN"' EXIT

if [[ -n "${SIMDECK_BUILD_TARGET:-}" ]]; then
  TARGET="$SIMDECK_BUILD_TARGET"

  if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "Installing missing Rust target: $TARGET"
    rustup target add "$TARGET"
  fi

  cargo build --release --manifest-path "$MANIFEST_PATH" --target "$TARGET"
  SERVER_BIN="$SERVER_TARGET_DIR/$TARGET/release/simdeck-server"
else
  cargo build --release --manifest-path "$MANIFEST_PATH"
  SERVER_BIN="$SERVER_TARGET_DIR/release/simdeck-server"
fi

cp "$SERVER_BIN" "$TMP_OUTPUT_BIN"
chmod +x "$TMP_OUTPUT_BIN"
mv -f "$TMP_OUTPUT_BIN" "$OUTPUT_BIN"
trap - EXIT

echo "Built $OUTPUT_BIN"
file "$OUTPUT_BIN"

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
