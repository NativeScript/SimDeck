#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT="$BUILD_DIR/simdeck"
OUTPUT_BIN="$BUILD_DIR/simdeck-bin"
MANIFEST_PATH="$ROOT_DIR/server/Cargo.toml"
SERVER_BIN="$ROOT_DIR/server/target/release/simdeck-server"

mkdir -p "$BUILD_DIR"

cargo build --release --manifest-path "$MANIFEST_PATH"
TMP_OUTPUT_BIN="$OUTPUT_BIN.tmp.$$"
cp "$SERVER_BIN" "$TMP_OUTPUT_BIN"
chmod +x "$TMP_OUTPUT_BIN"
mv -f "$TMP_OUTPUT_BIN" "$OUTPUT_BIN"

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
