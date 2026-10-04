#!/bin/bash
# Installs what `cargo test`, `cargo clippy`, `npm test` and `npm run typecheck`
# need in a Claude Code cloud session. Safe to run more than once.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}"

# System libraries for the Tauri app (same list as .github/workflows/ci.yml).
packages=(
  libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev
  librsvg2-dev libpipewire-0.3-dev libspa-0.2-dev libxcb1-dev libxrandr-dev
  libdbus-1-dev libegl-dev libwayland-dev libgbm-dev libasound2-dev libclang-dev
  pkg-config
)
missing=()
for p in "${packages[@]}"; do
  dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q "install ok installed" || missing+=("$p")
done
if [ ${#missing[@]} -gt 0 ]; then
  sudo=""
  [ "$(id -u)" -ne 0 ] && sudo="sudo"
  $sudo apt-get update -qq
  DEBIAN_FRONTEND=noninteractive $sudo apt-get install -y -qq --no-install-recommends "${missing[@]}"
fi

# Frontend (vitest, tsc, vite).
npm install --no-audit --no-fund

# Rust: clippy for linting, then build the tests once so the cached container
# starts warm.
rustup component add clippy >/dev/null 2>&1 || true
cargo fetch
cargo test --workspace --no-run --quiet
