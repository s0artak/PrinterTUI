#!/usr/bin/env bash
# Development setup for PrinterTUI on Arch Linux (to just use the app, run install.sh instead).
#   from a checkout:  bash test/dev-setup-arch.sh
#   from scratch:     curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/test/dev-setup-arch.sh | bash
#
# What it does and where things go:
#   1. pacman: git, gcc, rust (unless cargo exists), cups, cups-filters, imagemagick
#   2. starts CUPS (cups.socket) so printers work
#   3. source code  -> the checkout this script is in, else cloned into ~/Development/PrinterTUI
#   4. build        -> <source>/target/release/printertui
#   5. command      -> /usr/local/bin/printertui-dev, a symlink to that build.
#                      `printertui` stays whatever install.sh put there; `printertui-dev` runs your
#                      build (rebuild with `cargo build --release`). Both share ~/.config/printertui.
set -euo pipefail

command -v pacman >/dev/null || {
	echo "error: this script is for Arch Linux" >&2
	exit 1
}

pkgs=(git gcc cups cups-filters imagemagick)
# rust conflicts with rustup, only add it when there is no cargo yet
command -v cargo >/dev/null || pkgs+=(rust)
sudo pacman -S --needed --noconfirm "${pkgs[@]}"
sudo systemctl enable --now cups.socket

src=${BASH_SOURCE[0]:-}
if [[ -f $src ]] && dir=$(git -C "$(dirname "$src")" rev-parse --show-toplevel 2>/dev/null); then
	: # run from a checkout: build that one, leave its git state alone
else
	dir="$HOME/Development/PrinterTUI"
	if [[ -d $dir/.git ]]; then
		git -C "$dir" pull --ff-only || true
	else
		mkdir -p "$(dirname "$dir")"
		git clone https://github.com/s0artak/PrinterTUI "$dir"
	fi
fi

cd "$dir"
cargo build --release
sudo ln -sf "$dir/target/release/printertui" /usr/local/bin/printertui-dev
echo "Built $dir/target/release/printertui, run: printertui-dev"
