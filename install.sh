#!/usr/bin/env sh
set -eu

REPO="raihan0x1/0x1-ai-memory"
INSTALL_DIR="${OX1_INSTALL_DIR:-$HOME/.local/bin}"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) OS_KEY="darwin" ;;
  Linux) OS_KEY="linux" ;;
  *)
    echo "Unsupported OS: $OS (build from source with cargo build --release)" >&2
    exit 1
    ;;
esac

case "$ARCH" in
  arm64|aarch64) ARCH_KEY="arm64" ;;
  x86_64|amd64) ARCH_KEY="x64" ;;
  *)
    echo "Unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

ASSET="0x1-memory-${OS_KEY}-${ARCH_KEY}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/latest/download"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${ASSET} from ${REPO}..."
curl -fsSL "${BASE_URL}/${ASSET}" -o "${TMP_DIR}/${ASSET}"
curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "${TMP_DIR}/SHA256SUMS.txt"

EXPECTED="$(grep " ${ASSET}\$" "${TMP_DIR}/SHA256SUMS.txt" | awk '{print $1}')"
if [ -n "$EXPECTED" ]; then
  if command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ASSET}" | awk '{print $1}')"
  elif command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{print $1}')"
  else
    ACTUAL=""
  fi
  if [ -n "$ACTUAL" ] && [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "Checksum mismatch for ${ASSET}!" >&2
    exit 1
  fi
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "${TMP_DIR}/${ASSET}" -C "$TMP_DIR"
mv "${TMP_DIR}/0x1-memory" "${INSTALL_DIR}/0x1-memory"
chmod +x "${INSTALL_DIR}/0x1-memory"

echo ""
echo "Installed 0x1-memory to ${INSTALL_DIR}/0x1-memory"
echo "Make sure ${INSTALL_DIR} is in your PATH, then run:"
echo "  0x1-memory serve --open"
echo "  0x1-memory setup"
