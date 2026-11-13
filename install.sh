#!/bin/sh
# Build and install hx from the yaa110/helix source tree.
#
#   curl -fsSL https://raw.githubusercontent.com/yaa110/helix/main/install.sh | sh
#
# This intentionally does not depend on GitHub Releases: the source is
# downloaded, compiled locally with Cargo, and the runtime is installed next to
# the binary.
#
# Optional environment variables:
#   HELIX_INSTALL_DIR   install prefix (default: $HOME/.local)
#   HELIX_BRANCH        source branch to build (default: main)
#   HELIX_REPO          GitHub repo (default: yaa110/helix)
#   HELIX_SKIP_GRAMMARS set to 1 to skip fetching/building tree-sitter grammars

set -eu

repo="${HELIX_REPO:-yaa110/helix}"
branch="${HELIX_BRANCH:-main}"
prefix="${HELIX_INSTALL_DIR:-$HOME/.local}"

if ! command -v cargo >/dev/null 2>&1; then
  cat >&2 <<'EOF'
cargo was not found.

Install Rust (1.90 or newer) first:
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

Then restart your shell (or source ~/.cargo/env) and run this installer again.
EOF
  exit 1
fi

if [ "${HELIX_SKIP_GRAMMARS:-0}" != "1" ]; then
  missing=""
  for tool in git cc c++; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      missing="$missing $tool"
    fi
  done
  if [ -n "$missing" ]; then
    cat >&2 <<EOF
Missing build tools:$missing

Tree-sitter grammars need git and a C/C++ compiler. Install them, or rerun
with HELIX_SKIP_GRAMMARS=1 to install hx without grammars.
EOF
    exit 1
  fi
fi

source_url="https://github.com/$repo/archive/refs/heads/$branch.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $repo ($branch) source..."
curl -fL "$source_url" -o "$tmp/source.tar.gz"

mkdir -p "$tmp/src"
tar -xzf "$tmp/source.tar.gz" -C "$tmp/src" --strip-components=1

if [ "${HELIX_SKIP_GRAMMARS:-0}" = "1" ]; then
  echo "Building hx from source without tree-sitter grammars..."
  export HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1
else
  echo "Building hx and tree-sitter grammars from source (this can take a while)..."
fi

CARGO_TARGET_DIR="$tmp/target" \
  cargo install --locked --path "$tmp/src/helix-term" --root "$tmp/install"

mkdir -p "$prefix/bin" "$prefix/libexec/helix" "$prefix/share/helix"
install -m 755 "$tmp/install/bin/hx" "$prefix/libexec/helix/hx"

rm -rf "$prefix/share/helix/runtime"
cp -R "$tmp/src/runtime" "$prefix/share/helix/runtime"

cat > "$prefix/bin/hx" <<EOF
#!/bin/sh
HELIX_RUNTIME="\${HELIX_RUNTIME:-$prefix/share/helix/runtime}" exec "$prefix/libexec/helix/hx" "\$@"
EOF
chmod +x "$prefix/bin/hx"

if [ "${HELIX_SKIP_GRAMMARS:-0}" = "1" ]; then
  echo "Skipped tree-sitter grammars (HELIX_SKIP_GRAMMARS=1)."
  echo "You can build them later with:"
  echo "  HELIX_RUNTIME=\"$prefix/share/helix/runtime\" $prefix/libexec/helix/hx --grammar fetch"
  echo "  HELIX_RUNTIME=\"$prefix/share/helix/runtime\" $prefix/libexec/helix/hx --grammar build"
fi

echo
echo "Installed hx to $prefix/bin/hx"
echo "Runtime: $prefix/share/helix/runtime"
"$prefix/bin/hx" --version
case ":$PATH:" in
  *":$prefix/bin:"*) ;;
  *)
    echo
    echo "Add this to your shell profile:"
    echo "  export PATH=\"$prefix/bin:\$PATH\""
    ;;
esac
