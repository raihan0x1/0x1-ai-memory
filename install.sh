#!/usr/bin/env sh
set -eu

REPO="raihan0x1/0x1-ai-memory"

if [ -n "${OX1_INSTALL_DIR:-}" ]; then
  INSTALL_DIR="$OX1_INSTALL_DIR"
else
  case ":${PATH:-}:" in
    *":$HOME/.local/bin:"*) INSTALL_DIR="$HOME/.local/bin" ;;
    *)
      if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
        INSTALL_DIR="/usr/local/bin"
      else
        INSTALL_DIR="$HOME/.local/bin"
      fi
      ;;
  esac
fi

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS:$ARCH" in
  Darwin:arm64|Darwin:aarch64) ASSET="0x1-memory-darwin-arm64.tar.gz" ;;
  Linux:x86_64|Linux:amd64)    ASSET="0x1-memory-linux-x64.tar.gz" ;;
  *)
    echo "No prebuilt binary for ${OS}/${ARCH} yet. Build from source:" >&2
    echo "  git clone https://github.com/${REPO}.git && cd 0x1-ai-memory && (cd ui && pnpm install && pnpm build) && cargo build --release" >&2
    exit 1
    ;;
esac

BASE_URL="https://github.com/${REPO}/releases/latest/download"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${ASSET} from ${REPO}..."
curl -fsSL "${BASE_URL}/${ASSET}" -o "${TMP_DIR}/${ASSET}"

if [ "${OX1_SKIP_CHECKSUM:-0}" != "1" ]; then
  curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "${TMP_DIR}/SHA256SUMS.txt"
  EXPECTED="$(awk -v file="$ASSET" '$2 == file || $2 == ("*" file) { print $1; exit }' "${TMP_DIR}/SHA256SUMS.txt")"
  if [ -z "$EXPECTED" ]; then
    echo "Checksum entry missing for ${ASSET} in SHA256SUMS.txt (aborting)." >&2
    exit 1
  fi
  if command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ASSET}" | awk '{print $1}')"
  elif command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{print $1}')"
  else
    echo "Neither shasum nor sha256sum found; set OX1_SKIP_CHECKSUM=1 to bypass." >&2
    exit 1
  fi
  if [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "Checksum mismatch for ${ASSET}! Expected ${EXPECTED}, got ${ACTUAL}." >&2
    exit 1
  fi
  echo "Verified SHA-256 checksum."
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "${TMP_DIR}/${ASSET}" -C "$TMP_DIR"
mv "${TMP_DIR}/0x1-memory" "${INSTALL_DIR}/0x1-memory"
chmod +x "${INSTALL_DIR}/0x1-memory"

echo ""
echo "Installed 0x1-memory to ${INSTALL_DIR}/0x1-memory"

case ":${PATH:-}:" in
  *":${INSTALL_DIR}:"*)
    echo "Run to start daemon & open Setup Wizard:"
    echo "  0x1-memory"
    ;;
  *)
    SHELL_RC="$HOME/.profile"
    case "${SHELL:-}" in
      */zsh)  SHELL_RC="$HOME/.zshrc" ;;
      */bash) SHELL_RC="$HOME/.bashrc" ;;
    esac
    echo ""
    echo "${INSTALL_DIR} is not in your PATH yet. Add it once:"
    echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ${SHELL_RC} && source ${SHELL_RC}"
    echo ""
    echo "Or start right now:"
    echo "  ${INSTALL_DIR}/0x1-memory"
    ;;
esac
