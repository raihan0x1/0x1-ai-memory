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

echo "==> Checking shell scripts, git hooks, and npm launcher"
sh -n install.sh
bash -n scripts/*.sh .githooks/*
node --check npm/bin/0x1-memory.mjs
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck install.sh scripts/*.sh .githooks/*
else
  echo "  (shellcheck not installed locally; syntax verified with sh/bash -n)"
fi

echo "==> Checking install.sh & npm launcher <-> release.yml asset parity"
while IFS= read -r asset; do
  stem="${asset%.tar.gz}"
  stem="${stem%.zip}"
  if ! grep -Fq "asset: ${stem}" .github/workflows/release.yml; then
    echo "ERROR: installer references '${asset}' but release.yml has no 'asset: ${stem}'" >&2
    exit 1
  fi
done < <(
  {
    sed -n 's/.*ASSET="\([^"]*\)".*/\1/p' install.sh
    sed -n "s/.*'\(0x1-memory-[^']*\)'.*/\1/p" npm/bin/0x1-memory.mjs
  } | sort -u
)

echo "==> Checking version parity (Cargo.toml, ui/package.json, npm/package.json)"
python3 - <<'PY'
import json, pathlib, re
cargo_m = re.search(r'(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"', pathlib.Path("Cargo.toml").read_text())
assert cargo_m, "Missing version in Cargo.toml"
versions = {
    "Cargo.toml": cargo_m.group(1),
    "ui/package.json": json.loads(pathlib.Path("ui/package.json").read_text())["version"],
    "npm/package.json": json.loads(pathlib.Path("npm/package.json").read_text())["version"],
}
if len(set(versions.values())) != 1:
    raise SystemExit(f"Version mismatch across manifests: {versions}")
PY

echo "==> Checking Rim UI guardrails (no raw <select, valid number input steps)"
if grep -rnE '<select([[:space:]]|>)' ui/src/pages/; then
  echo "ERROR: Use <RimSelect /> instead of native <select> in ui/src/pages/" >&2
  exit 1
fi
python3 - <<'PY'
import pathlib, re
for path in sorted(pathlib.Path("ui/src/pages").glob("*.tsx")):
    text = path.read_text()
    for m in re.finditer(r'<input\b[\s\S]*?/>', text):
        tag = m.group(0)
        if 'type="number"' not in tag:
            continue
        min_m = re.search(r'\bmin=\{([^}]+)\}', tag)
        step_m = re.search(r'\bstep=(?:\{([^}]+)\}|"([^"]+)")', tag)
        if min_m and step_m:
            raw_step = step_m.group(1) or step_m.group(2)
            if raw_step != "any":
                min_v, step_v = float(min_m.group(1)), float(raw_step)
                if step_v > 1 and (min_v % step_v) != 0:
                    raise SystemExit(f"HTML5 stepMismatch trap in {path}: min={min_v}, step={step_v}")
PY

echo "==> Checking SonarCloud static guardrails"
python3 scripts/sonar-guard.py

if [[ "$SKIP_UI" -eq 0 ]]; then
  echo "==> Linting & building embedded UI"
  (cd ui && pnpm lint && pnpm build)
fi

echo "==> Checking Rust formatting, Clippy, and tests (offline ONNX)"
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
OX1_MEMORY_DISABLE_ONNX=1 cargo test --all-targets

echo "All checks passed."
