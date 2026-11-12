#!/bin/sh
# Install the prebuilt hx binary from the yaa110/helix GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/yaa110/helix/master/install.sh | sh
#
# Optional environment variables:
#   HELIX_INSTALL_DIR  install prefix (default: $HOME/.local)
#   HELIX_VERSION      release tag, e.g. v0.1.0 (default: latest)
#   HELIX_REPO         GitHub repo (default: yaa110/helix)

set -eu

repo="${HELIX_REPO:-yaa110/helix}"
version="${HELIX_VERSION:-latest}"
prefix="${HELIX_INSTALL_DIR:-$HOME/.local}"

os="$(uname -s)"
arch="$(uname -m)"

case "$os-$arch" in
  Darwin-arm64|Darwin-aarch64)
    target="aarch64-apple-darwin"
    ;;
  Darwin-x86_64)
    target="x86_64-apple-darwin"
    ;;
  Linux-x86_64|Linux-amd64)
    target="x86_64-unknown-linux-gnu"
    ;;
  Linux-aarch64|Linux-arm64)
    target="aarch64-unknown-linux-gnu"
    ;;
  *)
    echo "unsupported platform: $os $arch" >&2
    exit 1
    ;;
esac

asset="hx-$target.tar.gz"
if [ "$version" = "latest" ]; then
  base="https://github.com/$repo/releases/latest/download"
else
  base="https://github.com/$repo/releases/download/$version"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $asset from $repo ($version)..."
curl -fL "$base/$asset" -o "$tmp/$asset"

if curl -fsL "$base/$asset.sha256" -o "$tmp/$asset.sha256"; then
  (
    cd "$tmp"
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum -c "$asset.sha256"
    else
      shasum -a 256 -c "$asset.sha256"
    fi
  )
fi

tar -xzf "$tmp/$asset" -C "$tmp"

mkdir -p "$prefix/bin" "$prefix/libexec/helix" "$prefix/share/helix"

install -m 755 "$tmp/hx" "$prefix/libexec/helix/hx"
rm -rf "$prefix/share/helix/runtime"
mv "$tmp/runtime" "$prefix/share/helix/runtime"

cat > "$prefix/bin/hx" <<EOF
#!/bin/sh
HELIX_RUNTIME="\${HELIX_RUNTIME:-$prefix/share/helix/runtime}" exec "$prefix/libexec/helix/hx" "\$@"
EOF
chmod +x "$prefix/bin/hx"

echo
echo "Installed hx to $prefix/bin/hx"
echo "Runtime: $prefix/share/helix/runtime"
case ":$PATH:" in
  *":$prefix/bin:"*) ;;
  *)
    echo
    echo "Add this to your shell profile:"
    echo "  export PATH=\"$prefix/bin:\$PATH\""
    ;;
esac
