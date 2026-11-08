#!/usr/bin/env bash
# Start rubato-server in dev mode with the map from the local file, run Playwright, stop it.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd .. && pwd)"
BIN="${RUBATO_SERVER_BIN:-$ROOT/target/debug/rubato-server}"
[ -x "$BIN" ] || { echo "build the server first: cargo build -p rubato-server" >&2; exit 2; }
[ -f dist/index.html ] || { echo "build the web first: pnpm build" >&2; exit 2; }
TMP="$(mktemp -d)"
trap 'kill "$PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT
RUBATO_BIND=127.0.0.1:18090 RUBATO_PUBLIC_URL=http://127.0.0.1:18090 RUBATO_DEV_USER=m0x6A \
  RUBATO_DATABASE_URL="sqlite://$TMP/e2e.db?mode=rwc" RUBATO_MAP_FILE="$ROOT/docs/design/rubato-map.yml" \
  RUBATO_WEB_DIST="$PWD/dist" RUBATO_GITHUB_POLL_SECS=3600 RUST_LOG=info \
  "$BIN" > "$TMP/server.log" 2>&1 &
PID=$!
for _ in $(seq 1 50); do
  curl -sf http://127.0.0.1:18090/api/health > /dev/null 2>&1 && break
  sleep 0.2
done
curl -sf http://127.0.0.1:18090/api/health > /dev/null || { echo "server did not start"; cat "$TMP/server.log"; exit 1; }
RUBATO_E2E_URL=http://127.0.0.1:18090 pnpm exec playwright test "$@"
