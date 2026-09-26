#!/bin/sh
# Installs the latest Orqadence release as $ORQADENCE_INSTALL_DIR/orqa
# (default ~/.local/bin). Keep that directory writable by you: the binary
# updates itself in place from then on.
#   curl -fsSL https://raw.githubusercontent.com/muresanroland/orqadence/main/install.sh | sh
set -eu

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  *)
    echo "orqa: no release binary for $(uname -s) $(uname -m); build from source:" >&2
    echo "  cargo install --git https://github.com/muresanroland/orqadence" >&2
    exit 1
    ;;
esac

dir=${ORQADENCE_INSTALL_DIR:-$HOME/.local/bin}
mkdir -p "$dir"
tmp="$dir/.orqa.download.$$"
trap 'rm -f "$tmp"' EXIT
curl -fsSL "https://github.com/muresanroland/orqadence/releases/latest/download/orqa-$target" -o "$tmp"
chmod +x "$tmp"
mv -f "$tmp" "$dir/orqa" # replaces a symlink too, never follows it
echo "installed $("$dir/orqa" --version) as $dir/orqa"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "add $dir to your PATH" ;;
esac
