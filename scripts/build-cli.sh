#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_DIR="$ROOT_DIR/build"
OUTPUT="$BUILD_DIR/xcode-canvas-web"
OUTPUT_BIN="$BUILD_DIR/xcode-canvas-web-bin"
MANIFEST_PATH="$ROOT_DIR/server/Cargo.toml"
SERVER_BIN="$ROOT_DIR/server/target/release/xcode-canvas-web-server"

mkdir -p "$BUILD_DIR"

cargo build --release --manifest-path "$MANIFEST_PATH"
cp "$SERVER_BIN" "$OUTPUT_BIN"
chmod +x "$OUTPUT_BIN"

cat > "$OUTPUT" <<EOF
#!/usr/bin/env zsh
set -euo pipefail

SCRIPT_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
exec "\$SCRIPT_DIR/$(basename "$OUTPUT_BIN")" "\$@"
EOF
chmod +x "$OUTPUT"

echo "Built $OUTPUT"
