#!/bin/sh
# Install Remote Cast (chrome-stream-mcp) plugin from GitHub Releases with SHA256 verification.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/remote-cast/main/scripts/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/remote-cast/main/scripts/install.sh | sh -s -- --reload
#   curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/remote-cast/main/scripts/install.sh | sh -s -- --uninstall
#   CHROME_STREAM_VERSION=v0.5.5 sh scripts/install.sh

set -eu

REPO="${CHROME_STREAM_REPO:-eatingDream-antigravity-plugins/remote-cast}"
PLUGIN_ID="chrome-stream-mcp"
MCP_SERVER_NAME="${PLUGIN_ID}_${PLUGIN_ID}"
PLUGINS_DIR="${ANTIGRAVITY_PLUGINS_DIR:-$HOME/.gemini/config/plugins}"
INSTALL_DIR="${PLUGINS_DIR}/${PLUGIN_ID}"
VERSION="${CHROME_STREAM_VERSION:-}"
UNINSTALL=0
RELOAD_MODE="auto"

is_single_line_match() {
  _val="$1"
  _re="$2"
  awk -v val="$_val" -v re="$_re" 'BEGIN {
    if (index(val, "\n") > 0 || index(val, "\r") > 0) exit 1;
    if (val ~ re) exit 0;
    exit 1;
  }'
}

canonicalize_path() {
  _p="$1"
  [ -n "$_p" ] || return 1
  while [ "$_p" != "/" ] && [ "${_p%/}" != "$_p" ]; do
    _p="${_p%/}"
  done
  if [ -e "$_p" ] || [ -L "$_p" ]; then
    if [ -d "$_p" ] && [ ! -L "$_p" ]; then
      (CDPATH= cd -P -- "$_p" 2>/dev/null && pwd -P) || printf '%s\n' "$_p"
      return 0
    fi
    _dir="$(dirname -- "$_p")"
    _base="$(basename -- "$_p")"
    if [ -d "$_dir" ]; then
      _cdir="$(CDPATH= cd -P -- "$_dir" 2>/dev/null && pwd -P)" || _cdir="$_dir"
      if [ "$_cdir" = "/" ]; then
        printf '/%s\n' "$_base"
      else
        printf '%s/%s\n' "$_cdir" "$_base"
      fi
      return 0
    fi
  else
    _dir="$(dirname -- "$_p")"
    _base="$(basename -- "$_p")"
    if [ -d "$_dir" ]; then
      _cdir="$(CDPATH= cd -P -- "$_dir" 2>/dev/null && pwd -P)" || _cdir="$_dir"
      if [ "$_base" = "." ]; then
        printf '%s\n' "$_cdir"
      elif [ "$_cdir" = "/" ]; then
        printf '/%s\n' "$_base"
      else
        printf '%s/%s\n' "$_cdir" "$_base"
      fi
      return 0
    fi
  fi
  printf '%s\n' "$_p"
}

ensure_no_symlink() {
  if [ -L "$1" ]; then
    echo "error: refusing to follow symlink at $1" >&2
    exit 1
  fi
}

install_staged_file() {
  _src="$1"
  _dst="$2"
  _mode="$3"
  ensure_no_symlink "$_dst"
  _tmp_dst="${_dst}.new.$$"
  ensure_no_symlink "$_tmp_dst"
  install -m "$_mode" "$_src" "$_tmp_dst"
  mv -f "$_tmp_dst" "$_dst"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version|-v)
      shift
      [ "$#" -gt 0 ] || { echo "error: --version requires a value" >&2; exit 1; }
      VERSION="$1"
      ;;
    --dir|-d|--dest)
      shift
      [ "$#" -gt 0 ] || { echo "error: --dir requires a value" >&2; exit 1; }
      INSTALL_DIR="$1"
      ;;
    --reload)
      RELOAD_MODE="force"
      ;;
    --no-reload)
      RELOAD_MODE="none"
      ;;
    --uninstall)
      UNINSTALL=1
      ;;
    --help|-h)
      cat <<'EOF'
Remote Cast (chrome-stream-mcp) installer

Options:
  -v, --version <tag>   Install a specific release tag (e.g. v0.5.5). Defaults to latest.
  -d, --dir <path>      Target plugin directory (default: ${ANTIGRAVITY_PLUGINS_DIR:-$HOME/.gemini/config/plugins}/chrome-stream-mcp).
      --reload          Outside Antigravity, toggle the plugin via `agy plugin disable/enable`.
      --no-reload       Skip automatic Antigravity MCP hot-reload after install.
      --uninstall       Remove the installed plugin directory.
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

if [ "$UNINSTALL" -eq 1 ]; then
  if [ -z "$INSTALL_DIR" ]; then
    echo "error: refusing to remove empty directory path" >&2
    exit 1
  fi
  ensure_no_symlink "$INSTALL_DIR"
  CANON_INSTALL_DIR="$(canonicalize_path "$INSTALL_DIR")"
  CANON_HOME="$(canonicalize_path "${HOME:-/}")"
  CANON_PLUGINS="$(canonicalize_path "$PLUGINS_DIR")"
  case "$CANON_INSTALL_DIR" in
    ""|"/"|"$CANON_HOME"|"$CANON_PLUGINS"|"/Users"|"/home"|"/tmp"|"/var"|"/etc"|"/usr")
      echo "error: refusing to remove unsafe directory: ${INSTALL_DIR}" >&2
      exit 1
      ;;
  esac
  if [ -d "$CANON_INSTALL_DIR" ]; then
    rm -rf "$CANON_INSTALL_DIR"
    echo "Removed Remote Cast (${PLUGIN_ID}) from ${CANON_INSTALL_DIR}"
    echo "Reload Antigravity to finish uninstalling."
  else
    echo "Remote Cast (${PLUGIN_ID}) is not installed at ${CANON_INSTALL_DIR}"
  fi
  exit 0
fi

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command '$1' not found in PATH" >&2
    exit 1
  fi
}

need_cmd curl
need_cmd tar
need_cmd awk
need_cmd install

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

if ! is_single_line_match "$VERSION" '^v[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.]+)?$'; then
  echo "error: invalid release version tag: ${VERSION}" >&2
  exit 1
fi

ARCHIVE="chrome-stream-mcp-${VERSION}-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

echo "Downloading ${ARCHIVE} (${VERSION})..."
curl -fsSL -o "${TMP_DIR}/${ARCHIVE}" "${BASE_URL}/${ARCHIVE}"
curl -fsSL -o "${TMP_DIR}/${ARCHIVE}.sha256" "${BASE_URL}/${ARCHIVE}.sha256"

EXPECTED_SHA="$(awk 'NR==1 {print $1; exit}' "${TMP_DIR}/${ARCHIVE}.sha256" | tr -d '\r\n')"
if ! is_single_line_match "$EXPECTED_SHA" '^[0-9a-fA-F]{64}$'; then
  echo "error: invalid SHA256 checksum format" >&2
  exit 1
fi
EXPECTED_SHA="$(printf '%s' "$EXPECTED_SHA" | tr 'A-F' 'a-f')"

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

ensure_no_symlink "$INSTALL_DIR"
for _sub in "" "/bin" "/panel" "/panel/dist" "/skills" "/skills/chrome-stream" "/assets" "/docs"; do
  ensure_no_symlink "${INSTALL_DIR}${_sub}"
done
mkdir -p "${INSTALL_DIR}/bin" "${INSTALL_DIR}/panel/dist" "${INSTALL_DIR}/skills/chrome-stream" "${INSTALL_DIR}/assets" "${INSTALL_DIR}/docs"

# Atomic binary swap avoids ETXTBSY if a persistent relay daemon is running.
install_staged_file "${STAGE_DIR}/bin/chrome-stream-mcp" "${INSTALL_DIR}/bin/chrome-stream-mcp" 0755

install_staged_file "${STAGE_DIR}/panel/dist/panel.html" "${INSTALL_DIR}/panel/dist/panel.html" 0644
install_staged_file "${STAGE_DIR}/plugin.json" "${INSTALL_DIR}/plugin.json" 0644
install_staged_file "${STAGE_DIR}/mcp_config.json" "${INSTALL_DIR}/mcp_config.json" 0644
install_staged_file "${STAGE_DIR}/skills/chrome-stream/SKILL.md" "${INSTALL_DIR}/skills/chrome-stream/SKILL.md" 0644
install_staged_file "${STAGE_DIR}/assets/logo.svg" "${INSTALL_DIR}/assets/logo.svg" 0644
if [ -f "${STAGE_DIR}/docs/ai-guide.md" ]; then
  install_staged_file "${STAGE_DIR}/docs/ai-guide.md" "${INSTALL_DIR}/docs/ai-guide.md" 0644
fi
if [ -f "${STAGE_DIR}/llms.txt" ]; then
  install_staged_file "${STAGE_DIR}/llms.txt" "${INSTALL_DIR}/llms.txt" 0644
fi
install_staged_file "${STAGE_DIR}/LICENSE" "${INSTALL_DIR}/LICENSE" 0644
install_staged_file "${STAGE_DIR}/README.md" "${INSTALL_DIR}/README.md" 0644

reload_host() {
  if [ "$RELOAD_MODE" = "none" ]; then
    echo "Skipped MCP hot-reload (--no-reload)."
    return 0
  fi
  if [ -n "${ANTIGRAVITY_CSRF_TOKEN:-}" ]; then
    LS_ADDR="${ANTIGRAVITY_LS_ADDRESS:-localhost:5387}"
    if ! is_single_line_match "$LS_ADDR" '^(localhost|127\.0\.0\.1|\[::1\]):[0-9]{1,5}$'; then
      echo "Note: ignoring unsafe ANTIGRAVITY_LS_ADDRESS; restart the MCP server from Customizations > Installed."
      return 0
    fi
    LS_PORT="${LS_ADDR##*:}"
    if [ "$LS_PORT" -lt 1 ] || [ "$LS_PORT" -gt 65535 ]; then
      echo "Note: ignoring invalid ANTIGRAVITY_LS_ADDRESS port; restart the MCP server from Customizations > Installed."
      return 0
    fi
    if ! awk -v t="$ANTIGRAVITY_CSRF_TOKEN" 'BEGIN {
      if (t == "" || t ~ /[\001-\037\177"\\]/) exit 1;
      exit 0;
    }'; then
      echo "Note: ignoring unsafe ANTIGRAVITY_CSRF_TOKEN; restart the MCP server from Customizations > Installed."
      return 0
    fi
    if printf 'header = "Content-Type: application/json"\nheader = "x-codeium-csrf-token: %s"\n' "${ANTIGRAVITY_CSRF_TOKEN}" | \
      curl -sfg --connect-timeout 2 --max-time 5 -K - \
      -d "{\"serverName\":\"${MCP_SERVER_NAME}\"}" \
      "http://${LS_ADDR}/exa.language_server_pb.LanguageServerService/RefreshMcpServers" >/dev/null 2>&1; then
      echo "Refreshed MCP server (${MCP_SERVER_NAME}) in Antigravity."
      echo "Close and reopen the Remote Cast panel tab to load the updated UI."
      return 0
    fi
    echo "Note: could not reach Antigravity RefreshMcpServers; restart the MCP server from Customizations > Installed."
    return 0
  fi
  if [ "$RELOAD_MODE" = "force" ]; then
    if command -v agy >/dev/null 2>&1; then
      if agy plugin disable "${PLUGIN_ID}" >/dev/null 2>&1 && agy plugin enable "${PLUGIN_ID}" >/dev/null 2>&1; then
        echo "Reloaded plugin ${PLUGIN_ID} via agy CLI."
        return 0
      fi
      echo "Note: agy plugin reload did not complete; restart the plugin from Customizations > Installed."
    else
      echo "Note: --reload requested, but 'agy' CLI was not found in PATH."
    fi
  fi
  return 0
}

echo ""
echo "Installed Remote Cast (${PLUGIN_ID}) ${VERSION} (${TARGET}) to:"
echo "  ${INSTALL_DIR}"
echo ""
reload_host
echo ""
echo "Next steps:"
echo "  1. Enable Chrome remote debugging at chrome://inspect/#remote-debugging (or click Open separate Chrome in the panel)"
echo "  2. Run: ${INSTALL_DIR}/bin/chrome-stream-mcp doctor"
if [ -z "${ANTIGRAVITY_CSRF_TOKEN:-}" ] && [ "$RELOAD_MODE" != "force" ]; then
  echo "  3. Reload Antigravity, or run: agy plugin disable ${PLUGIN_ID} && agy plugin enable ${PLUGIN_ID}"
fi
