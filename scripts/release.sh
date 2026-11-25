#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

DRY_RUN=0
SKIP_CHECKS=0
AUTO_YES=0
ALLOW_NON_MAIN=0
TARGET=""

usage() {
  cat <<'EOF'
Usage: ./scripts/release.sh [current|patch|minor|major|X.Y.Z] [options]

Interactive if no version argument is passed. Bumps Cargo.toml, Cargo.lock,
ui/package.json, and npm/package.json, runs ./scripts/check.sh, commits, tags
vX.Y.Z, and pushes to origin so .github/workflows/release.yml automatically
builds & publishes (GitHub Release + npm when NPM_TOKEN is configured).

Options:
  --dry-run         Preview actions without modifying files, tags, or remote
  --skip-checks     Skip ./scripts/check.sh before tagging
  -y, --yes         Skip confirmation prompt
  --allow-non-main  Allow releasing from a branch other than main
  -h, --help        Show this help
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --skip-checks) SKIP_CHECKS=1 ;;
    -y|--yes) AUTO_YES=1 ;;
    --allow-non-main) ALLOW_NON_MAIN=1 ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      if [[ -n "$TARGET" ]]; then
        echo "Unexpected extra argument: $1" >&2
        usage >&2
        exit 1
      fi
      TARGET="$1"
      ;;
  esac
  shift
done

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" != "main" ]] && [[ "$ALLOW_NON_MAIN" -eq 0 ]]; then
  echo "ERROR: Current branch is '${BRANCH}'. Switch to 'main' or pass --allow-non-main." >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "ERROR: Working tree has uncommitted changes. Commit or stash them first." >&2
  git status -s >&2
  exit 1
fi

echo "==> Syncing tags from origin"
git fetch --tags origin --quiet

AHEAD=0
if git rev-parse --verify "origin/${BRANCH}" >/dev/null 2>&1; then
  BEHIND="$(git rev-list --count "HEAD..origin/${BRANCH}")"
  if [[ "$BEHIND" -gt 0 ]]; then
    echo "ERROR: Local '${BRANCH}' is ${BEHIND} commit(s) behind origin/${BRANCH}. Pull/rebase first." >&2
    exit 1
  fi
  AHEAD="$(git rev-list --count "origin/${BRANCH}..HEAD")"
fi

CURRENT_VER="$(python3 -c '
import re, pathlib
text = pathlib.Path("Cargo.toml").read_text()
m = re.search(r"(?ms)^\[package\].*?^version\s*=\s*\"([^\"]+)\"", text)
if not m:
    raise SystemExit("Could not find [package] version in Cargo.toml")
print(m.group(1))
')"

LATEST_TAG="$(git tag --list 'v*' --sort=-v:refname | head -n 1 || true)"

IFS='.' read -r CUR_MAJOR CUR_MINOR CUR_PATCH_RAW <<< "$CURRENT_VER"
CUR_PATCH="${CUR_PATCH_RAW%%-*}"
NEXT_PATCH="${CUR_MAJOR}.${CUR_MINOR}.$((CUR_PATCH + 1))"
NEXT_MINOR="${CUR_MAJOR}.$((CUR_MINOR + 1)).0"
NEXT_MAJOR="$((CUR_MAJOR + 1)).0.0"

CURRENT_TAG_EXISTS=0
if git rev-parse -q --verify "refs/tags/v${CURRENT_VER}" >/dev/null 2>&1; then
  CURRENT_TAG_EXISTS=1
fi

if [[ -z "$TARGET" ]]; then
  echo ""
  echo "Current version : ${CURRENT_VER}"
  echo "Latest git tag  : ${LATEST_TAG:-none}"
  echo ""
  echo "Choose release version:"
  if [[ "$CURRENT_TAG_EXISTS" -eq 0 ]]; then
    echo "  1) current  -> v${CURRENT_VER}  (tag current version without bumping)"
    echo "  2) patch    -> v${NEXT_PATCH}"
    echo "  3) minor    -> v${NEXT_MINOR}"
    echo "  4) major    -> v${NEXT_MAJOR}"
    echo "  5) custom   -> enter X.Y.Z"
    printf "Select [1-5]: "
    read -r CHOICE
    case "$CHOICE" in
      1|current) TARGET="current" ;;
      2|patch)   TARGET="patch" ;;
      3|minor)   TARGET="minor" ;;
      4|major)   TARGET="major" ;;
      5|custom)
        printf "Enter version (e.g. 0.2.0 or 0.2.0-rc.1): "
        read -r TARGET
        ;;
      *)
        echo "Aborted." >&2
        exit 1
        ;;
    esac
  else
    echo "  1) patch    -> v${NEXT_PATCH}"
    echo "  2) minor    -> v${NEXT_MINOR}"
    echo "  3) major    -> v${NEXT_MAJOR}"
    echo "  4) custom   -> enter X.Y.Z"
    printf "Select [1-4]: "
    read -r CHOICE
    case "$CHOICE" in
      1|patch)   TARGET="patch" ;;
      2|minor)   TARGET="minor" ;;
      3|major)   TARGET="major" ;;
      4|custom)
        printf "Enter version (e.g. 0.2.0 or 0.2.0-rc.1): "
        read -r TARGET
        ;;
      *)
        echo "Aborted." >&2
        exit 1
        ;;
    esac
  fi
fi

case "$TARGET" in
  current) NEXT_VER="$CURRENT_VER" ;;
  patch)   NEXT_VER="$NEXT_PATCH" ;;
  minor)   NEXT_VER="$NEXT_MINOR" ;;
  major)   NEXT_VER="$NEXT_MAJOR" ;;
  *)       NEXT_VER="${TARGET#v}" ;;
esac

if ! [[ "$NEXT_VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "ERROR: Invalid semantic version '${NEXT_VER}' (expected X.Y.Z or X.Y.Z-pre)." >&2
  exit 1
fi

TAG="v${NEXT_VER}"
if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null 2>&1; then
  echo "ERROR: Tag '${TAG}' already exists." >&2
  exit 1
fi

NEEDS_BUMP=1
if [[ "$NEXT_VER" == "$CURRENT_VER" ]]; then
  NEEDS_BUMP=0
fi

if [[ "$SKIP_CHECKS" -eq 1 ]]; then
  CHECKS_LABEL="skipped (--skip-checks)"
  SONAR_LABEL="skipped (--skip-checks)"
else
  CHECKS_LABEL="./scripts/check.sh"
  if [[ "$AHEAD" -gt 0 ]]; then
    SONAR_LABEL="push ${AHEAD} commit(s) to origin/${BRANCH} -> ./scripts/sonar-gate.sh --wait 240"
  else
    SONAR_LABEL="./scripts/sonar-gate.sh --wait 240 on HEAD"
  fi
fi

echo ""
echo "Release plan:"
echo "  Branch  : ${BRANCH}"
echo "  Version : ${CURRENT_VER} -> ${NEXT_VER}"
echo "  Tag     : ${TAG}"
if [[ "$NEEDS_BUMP" -eq 1 ]]; then
  echo "  Files   : Cargo.toml, Cargo.lock, ui/package.json, npm/package.json"
  echo "  Commit  : chore(release): ${TAG}"
else
  echo "  Files   : unchanged (tagging current version)"
fi
echo "  Checks  : ${CHECKS_LABEL}"
echo "  Sonar   : ${SONAR_LABEL}"
echo "  Push    : git push origin ${BRANCH} ${TAG}"
echo ""

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "[dry-run] No changes made."
  exit 0
fi

if [[ "$AUTO_YES" -eq 0 ]]; then
  printf "Proceed to release %s and trigger GitHub Release? [y/N]: " "$TAG"
  read -r CONFIRM
  case "$CONFIRM" in
    y|Y|yes|YES) ;;
    *)
      echo "Aborted."
      exit 1
      ;;
  esac
fi

if [[ "$SKIP_CHECKS" -eq 0 ]]; then
  ./scripts/check.sh
  CODE_HEAD_SHA="$(git rev-parse HEAD)"
  if [[ "$AHEAD" -gt 0 ]]; then
    echo "==> Pushing ${AHEAD} commit(s) to origin/${BRANCH} for SonarCloud analysis"
    git push origin "$BRANCH"
  fi
  echo "==> Waiting for SonarCloud Quality Gate on ${CODE_HEAD_SHA:0:7}"
  ./scripts/sonar-gate.sh --sha "$CODE_HEAD_SHA" --branch "$BRANCH" --wait 240
fi

if [[ "$NEEDS_BUMP" -eq 1 ]]; then
  echo "==> Updating version to ${NEXT_VER}"
  python3 - "$NEXT_VER" <<'PY'
import json, pathlib, re, sys

ver = sys.argv[1]

cargo_path = pathlib.Path("Cargo.toml")
cargo_text = cargo_path.read_text()
new_cargo, count = re.subn(
    r'(?ms)(^\[package\].*?^version\s*=\s*")[^"]+(")',
    lambda m: f"{m.group(1)}{ver}{m.group(2)}",
    cargo_text,
    count=1,
)
if count != 1:
    raise SystemExit("Failed to update [package] version in Cargo.toml")
cargo_path.write_text(new_cargo)

for rel_path in ("ui/package.json", "npm/package.json"):
    pkg_path = pathlib.Path(rel_path)
    pkg_data = json.loads(pkg_path.read_text())
    pkg_data["version"] = ver
    pkg_path.write_text(json.dumps(pkg_data, indent=2) + "\n")
PY
  cargo update --workspace --offline
fi

if [[ "$NEEDS_BUMP" -eq 1 ]]; then
  git add Cargo.toml Cargo.lock ui/package.json npm/package.json
  if [[ "$SKIP_CHECKS" -eq 0 ]]; then
    git commit --no-verify -m "chore(release): ${TAG}"
  else
    git commit -m "chore(release): ${TAG}"
  fi
fi

git tag -a "$TAG" -m "Release ${TAG}"
OX1_SKIP_SONAR_GATE=1 git push origin "$BRANCH" "$TAG"

REPO_URL="$(git remote get-url origin | sed -E 's#\.git$##; s#^git@github\.com:#https://github.com/#')"
echo ""
echo "Released ${TAG} and triggered GitHub Release workflow."
echo "  Actions : ${REPO_URL}/actions"
echo "  Release : ${REPO_URL}/releases/tag/${TAG}"
if command -v gh >/dev/null 2>&1; then
  echo "  Watch   : gh run watch"
fi
