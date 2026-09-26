#!/usr/bin/env bash
set -euo pipefail

dir="$HOME/Development/PrinterTUI"

command -v pacman >/dev/null || {
	echo "error: this script is for Arch Linux" >&2
	exit 1
}

pkgs=(git gcc cups cups-filters imagemagick)
# rust conflicts with rustup, only add it when there is no cargo yet
command -v cargo >/dev/null || pkgs+=(rust)
sudo pacman -S --needed --noconfirm "${pkgs[@]}"
sudo systemctl enable --now cups.socket

if [[ ! -d $dir/.git ]]; then
	mkdir -p "$(dirname "$dir")"
	git clone https://github.com/s0artak/PrinterTUI "$dir"
fi

cd "$dir"
cargo build --release
echo "Built $dir/target/release/printertui"
