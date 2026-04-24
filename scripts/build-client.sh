#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CLIENT_DIR="$ROOT_DIR/client"

if [[ ! -d "$CLIENT_DIR/node_modules" ]]; then
  npm install --prefix "$CLIENT_DIR"
fi

npm run --prefix "$CLIENT_DIR" build
