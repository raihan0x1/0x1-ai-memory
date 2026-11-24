#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

SKIP_UI=0
for arg in "$@"; do
  case "$arg" in
    --skip-ui) SKIP_UI=1 ;;
    *)
      echo "Usage: $0 [--skip-ui]" >&2
      exit 1
      ;;
  esac
done

echo "==> Checking shell scripts"
sh -n install.sh
bash -n scripts/*.sh
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck install.sh scripts/*.sh
else
  echo "  (shellcheck not installed locally; syntax verified with sh/bash -n)"
fi

echo "==> Checking install.sh <-> release.yml asset parity"
while IFS= read -r asset; do
  stem="${asset%.tar.gz}"
  stem="${stem%.zip}"
  if ! grep -Fq "asset: ${stem}" .github/workflows/release.yml; then
    echo "ERROR: install.sh references '${asset}' but release.yml has no 'asset: ${stem}'" >&2
    exit 1
  fi
done < <(sed -n 's/.*ASSET="\([^"]*\)".*/\1/p' install.sh)

echo "==> Checking Rim UI guardrails (no raw <select in pages)"
if grep -rnE '<select([[:space:]]|>)' ui/src/pages/; then
  echo "ERROR: Use <RimSelect /> instead of native <select> in ui/src/pages/" >&2
  exit 1
fi

if [ "$SKIP_UI" -eq 0 ]; then
  echo "==> Building embedded UI"
  (cd ui && pnpm build)
fi

echo "==> Checking Rust formatting, Clippy, and tests (offline ONNX)"
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
OX1_MEMORY_DISABLE_ONNX=1 cargo test --all-targets

echo "All checks passed."
