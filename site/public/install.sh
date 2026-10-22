#!/bin/sh
# Installs vcwl on Linux and macOS:  curl -fsSL https://vcwl.chteau.bzh/install.sh | sh
set -eu

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  *) echo "No prebuilt vcwl for $(uname -s) $(uname -m); build it from https://github.com/chteau/vcwl" >&2; exit 1 ;;
esac

dir="${VCWL_INSTALL:-$HOME/.local/bin}"
url="https://github.com/chteau/vcwl/releases/latest/download/vcwl-$target.tar.gz"

mkdir -p "$dir"
echo "Downloading $url"
curl -fsSL "$url" | tar xz -C "$dir" ./vcwl
chmod +x "$dir/vcwl"
[ "$(uname -s)" = Darwin ] && xattr -d com.apple.quarantine "$dir/vcwl" 2>/dev/null || true

echo "Installed $("$dir/vcwl" --version) to $dir"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add it to your PATH:  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile  (or ~/.zshrc), then open a new terminal" ;;
esac
command -v node >/dev/null || echo "Node.js 20+ is needed for vcwl run, vcwl test and prerendering: https://nodejs.org"
echo "Then: vcwl new my-app && cd my-app && vcwl dev"
