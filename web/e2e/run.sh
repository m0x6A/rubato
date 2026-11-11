#!/usr/bin/env bash
# Start rubato-server in dev mode against a seeded local git repository, run Playwright, stop it.
#
# The repository stands in for GitHub (RUBATO_LOCAL_REPOS): the map is read from its `main`, a
# plan with a Flows table sits on branch `issue/901-lens`, and open issues come from
# fixtures/issues.json (RUBATO_DEV_ISSUES). Real git underneath, so plans, the plan lens and map
# proposals run for real. The poll interval is short because plans and open issues arrive by poll.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(cd .. && pwd)"
BIN="${RUBATO_SERVER_BIN:-$ROOT/target/debug/rubato-server}"
[ -x "$BIN" ] || { echo "build the server first: cargo build -p rubato-server" >&2; exit 2; }
[ -f dist/index.html ] || { echo "build the web first: pnpm build" >&2; exit 2; }
TMP="$(mktemp -d)"
PID=""
trap '[ -n "$PID" ] && kill "$PID" 2>/dev/null || true; rm -rf "$TMP"' EXIT

# Seed: main holds the map; issue/901-lens adds the plan.
SEED="$TMP/seed"
g() { git -C "$SEED" -c user.name=e2e -c user.email=e2e@example.invalid "$@"; }
git init -q -b main "$SEED"
mkdir -p "$SEED/docs/design" "$SEED/docs/plans"
cp "$ROOT/docs/design/rubato-map.yml" "$SEED/docs/design/rubato-map.yml"
g add -A && g commit -q -m "map"
g switch -q -c issue/901-lens
cp e2e/fixtures/901.md "$SEED/docs/plans/901.md"
g add -A && g commit -q -m "plan for #901"
g switch -q main
git clone -q --bare "$SEED" "$TMP/rubato.git"

RUBATO_BIND=127.0.0.1:18090 RUBATO_PUBLIC_URL=http://127.0.0.1:18090 RUBATO_DEV_USER=m0x6A \
  RUBATO_DATABASE_URL="sqlite://$TMP/e2e.db?mode=rwc" \
  RUBATO_LOCAL_REPOS="m0x6A/rubato=$TMP/rubato.git" RUBATO_MAP_REPO=m0x6A/rubato \
  RUBATO_MAP_PATH=docs/design/rubato-map.yml RUBATO_MAP_REF=main \
  RUBATO_DEV_ISSUES="$PWD/e2e/fixtures/issues.json" \
  RUBATO_WEB_DIST="$PWD/dist" RUBATO_GITHUB_POLL_SECS=5 RUST_LOG=info \
  "$BIN" > "$TMP/server.log" 2>&1 &
PID=$!
for _ in $(seq 1 50); do
  curl -sf http://127.0.0.1:18090/api/health > /dev/null 2>&1 && break
  sleep 0.2
done
curl -sf http://127.0.0.1:18090/api/health > /dev/null || { echo "server did not start"; cat "$TMP/server.log"; exit 1; }
# The first poll tick loads the map and lists the open issues.
for _ in $(seq 1 50); do
  curl -sf http://127.0.0.1:18090/api/overview 2>/dev/null | grep -q '"number":902' && break
  sleep 0.2
done
curl -sf http://127.0.0.1:18090/api/overview | grep -q '"number":902' || { echo "the backlog was not listed"; cat "$TMP/server.log"; exit 1; }
RUBATO_E2E_URL=http://127.0.0.1:18090 pnpm exec playwright test "$@" || { status=$?; echo "--- server log ---"; tail -n 60 "$TMP/server.log"; exit "$status"; }
