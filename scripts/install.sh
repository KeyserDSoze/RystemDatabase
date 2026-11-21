#!/usr/bin/env bash
set -euo pipefail

REPO="KeyserDSoze/RystemDatabase"
VERSION="${RYSTEMDB_VERSION:-latest}"
DEST="${RYSTEMDB_INSTALL_DIR:-$HOME/.local/bin}"
case "$(uname -s)" in
  Linux) os="linux" ;;
  Darwin) os="darwin" ;;
  *) echo "Unsupported OS" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch="x64" ;;
  aarch64|arm64) arch="arm64" ;;
  *) echo "Unsupported CPU architecture" >&2; exit 1 ;;
esac
asset="rystemdb-$os-$arch"
if [[ "$VERSION" == "latest" ]]; then
  VERSION="$(curl -fsSL -H 'Accept: application/vnd.github+json' -H 'User-Agent: rystemdb-installer' "https://api.github.com/repos/$REPO/releases?per_page=1" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
fi
if ! [[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]]; then
  echo "Invalid or missing release version: $VERSION" >&2; exit 1
fi
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
base="https://github.com/$REPO/releases/download/$VERSION"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"
curl -fsSL "$base/$asset" -o "$tmp/$asset"
expected="$(awk -v file="$asset" '$2==file {print $1}' "$tmp/SHA256SUMS")"
if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ ]]; then
  echo "Missing SHA-256 checksum for $asset" >&2; exit 1
fi
if command -v sha256sum >/dev/null; then
  actual="$(sha256sum "$tmp/$asset" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')"
fi
actual_lower="$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')"
expected_lower="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
if [[ "$actual_lower" != "$expected_lower" ]]; then
  echo "SHA-256 checksum mismatch" >&2; exit 1
fi
mkdir -p "$DEST"
install -m 755 "$tmp/$asset" "$DEST/rystemdb"
echo "Installed Rystem Database $VERSION at $DEST/rystemdb"
echo "Add $DEST to your PATH if needed."
