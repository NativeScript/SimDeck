#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CLIENT_DIR="$ROOT_DIR/client"

if [[ ! -d "$CLIENT_DIR/node_modules" ]]; then
  npm install --prefix "$CLIENT_DIR"
fi

npm run --prefix "$CLIENT_DIR" build

DEVTOOLS_FRONTEND="$ROOT_DIR/node_modules/@react-native/debugger-frontend/dist/third-party/front_end"
if [[ -d "$DEVTOOLS_FRONTEND" ]]; then
  rm -rf "$CLIENT_DIR/dist/chrome-devtools-ui"
  mkdir -p "$CLIENT_DIR/dist/chrome-devtools-ui"
  cp -R "$DEVTOOLS_FRONTEND/." "$CLIENT_DIR/dist/chrome-devtools-ui/"
fi
