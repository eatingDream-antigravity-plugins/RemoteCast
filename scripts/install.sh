#!/bin/sh
# Install Chrome Stream MCP plugin from GitHub Releases with SHA256 verification.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
#   CHROME_STREAM_VERSION=v0.1.0 sh scripts/install.sh

set -eu

REPO="${CHROME_STREAM_REPO:-eatingDream-antigravity-plugins/chrome-stream-mcp}"
PLUGINS_DIR="${ANTIGRAVITY_PLUGINS_DIR:-$HOME/.gemini/config/plugins}"
INSTALL_DIR="${PLUGINS_DIR}/chrome-stream-mcp"
VERSION="${CHROME_STREAM_VERSION:-}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version|-v)
      shift
      [ "$#" -gt 0 ] || { echo "error: --version requires a value" >&2; exit 1; }
      VERSION="$1"
      ;;
    --dir|-d)
      shift
      [ "$#" -gt 0 ] || { echo "error: --dir requires a value" >&2; exit 1; }
      INSTALL_DIR="$1"
      ;;
    --help|-h)
      cat <<'EOF'
Chrome Stream MCP installer

Options:
  -v, --version <tag>   Install a specific release tag (e.g. v0.1.0). Defaults to latest.
  -d, --dir <path>      Target plugin directory (default: ${ANTIGRAVITY_PLUGINS_DIR:-$HOME/.gemini/config/plugins}/chrome-stream-mcp).
  -h, --help            Show this help message.
EOF
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 1
      ;;
  esac
  shift
done

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command '$1' not found in PATH" >&2
    exit 1
  fi
}

need_cmd curl
need_cmd tar

OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}:${ARCH}" in
  Darwin:arm64|Darwin:aarch64)
    TARGET="aarch64-apple-darwin"
    ;;
  Darwin:x86_64|Darwin:amd64)
    TARGET="x86_64-apple-darwin"
    ;;
  Linux:x86_64|Linux:amd64)
    TARGET="x86_64-unknown-linux-gnu"
    ;;
  *)
    echo "error: unsupported prebuilt target ${OS}/${ARCH}." >&2
    echo "Build from source instead: https://github.com/${REPO}#build-from-source" >&2
    exit 1
    ;;
esac

if [ -z "$VERSION" ]; then
  EFFECTIVE_URL="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
  VERSION="${EFFECTIVE_URL##*/}"
  if [ -z "$VERSION" ] || [ "$VERSION" = "latest" ] || [ "$VERSION" = "releases" ]; then
    echo "error: could not determine latest release tag for ${REPO}." >&2
    echo "Pass CHROME_STREAM_VERSION=vX.Y.Z or build from source." >&2
    exit 1
  fi
else
  case "$VERSION" in
    v*) ;;
    *) VERSION="v${VERSION}" ;;
  esac
fi

ARCHIVE="chrome-stream-mcp-${VERSION}-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

echo "Downloading ${ARCHIVE} (${VERSION})..."
curl -fsSL -o "${TMP_DIR}/${ARCHIVE}" "${BASE_URL}/${ARCHIVE}"
curl -fsSL -o "${TMP_DIR}/${ARCHIVE}.sha256" "${BASE_URL}/${ARCHIVE}.sha256"

EXPECTED_SHA="$(awk '{print $1}' "${TMP_DIR}/${ARCHIVE}.sha256" | tr -d '\r\n')"
if [ -z "$EXPECTED_SHA" ]; then
  echo "error: empty SHA256 checksum file" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SHA="$(sha256sum "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL_SHA="$(shasum -a 256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}')"
elif command -v openssl >/dev/null 2>&1; then
  ACTUAL_SHA="$(openssl dgst -sha256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $NF}')"
else
  echo "error: no SHA256 tool found (need sha256sum, shasum, or openssl)" >&2
  exit 1
fi

if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
  echo "error: SHA256 mismatch for ${ARCHIVE}" >&2
  echo "  expected: ${EXPECTED_SHA}" >&2
  echo "  actual:   ${ACTUAL_SHA}" >&2
  exit 1
fi

echo "SHA256 verified: ${ACTUAL_SHA}"

STAGE_DIR="${TMP_DIR}/stage"
mkdir -p "$STAGE_DIR"
tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$STAGE_DIR"

mkdir -p "${INSTALL_DIR}/bin" "${INSTALL_DIR}/panel/dist" "${INSTALL_DIR}/skills/chrome-stream" "${INSTALL_DIR}/assets"

# Atomic binary swap avoids ETXTBSY if a persistent relay daemon is running.
cp "${STAGE_DIR}/bin/chrome-stream-mcp" "${INSTALL_DIR}/bin/chrome-stream-mcp.new"
chmod 0755 "${INSTALL_DIR}/bin/chrome-stream-mcp.new"
mv -f "${INSTALL_DIR}/bin/chrome-stream-mcp.new" "${INSTALL_DIR}/bin/chrome-stream-mcp"

cp "${STAGE_DIR}/panel/dist/panel.html" "${INSTALL_DIR}/panel/dist/panel.html"
cp "${STAGE_DIR}/plugin.json" "${INSTALL_DIR}/plugin.json"
cp "${STAGE_DIR}/mcp_config.json" "${INSTALL_DIR}/mcp_config.json"
cp "${STAGE_DIR}/skills/chrome-stream/SKILL.md" "${INSTALL_DIR}/skills/chrome-stream/SKILL.md"
cp "${STAGE_DIR}/assets/logo.svg" "${INSTALL_DIR}/assets/logo.svg"
cp "${STAGE_DIR}/LICENSE" "${INSTALL_DIR}/LICENSE"
cp "${STAGE_DIR}/README.md" "${INSTALL_DIR}/README.md"

echo ""
echo "Installed Chrome Stream MCP ${VERSION} (${TARGET}) to:"
echo "  ${INSTALL_DIR}"
echo ""
echo "Next steps:"
echo "  1. Enable Chrome remote debugging at chrome://inspect/#remote-debugging"
echo "  2. Run: ${INSTALL_DIR}/bin/chrome-stream-mcp doctor"
echo "  3. Restart or reload your MCP host to load the Chrome Stream plugin"
