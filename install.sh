#!/bin/sh
# Installs the latest harness release as $HARNESS_INSTALL_DIR/harness
# (default ~/.local/bin). Keep that directory writable by you: the binary
# updates itself in place from then on.
#   curl -fsSL https://raw.githubusercontent.com/muresanroland/harness/main/install.sh | sh
set -eu

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  *)
    echo "harness: no release binary for $(uname -s) $(uname -m); build from source:" >&2
    echo "  cargo install --git https://github.com/muresanroland/harness" >&2
    exit 1
    ;;
esac

dir=${HARNESS_INSTALL_DIR:-$HOME/.local/bin}
mkdir -p "$dir"
tmp="$dir/.harness.download.$$"
trap 'rm -f "$tmp"' EXIT
curl -fsSL "https://github.com/muresanroland/harness/releases/latest/download/harness-$target" -o "$tmp"
chmod +x "$tmp"
mv -f "$tmp" "$dir/harness" # replaces a symlink too, never follows it
echo "installed $("$dir/harness" --version) as $dir/harness"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "add $dir to your PATH" ;;
esac
