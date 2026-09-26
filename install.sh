#!/bin/sh
# PrinterTUI installer for Arch Linux and macOS.
#   install:   curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh
#   uninstall: curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh -s uninstall
set -eu

REPO=s0artak/PrinterTUI
BIN_DIR="$HOME/.local/bin"
BIN="$BIN_DIR/printertui"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

if [ "${1:-}" = uninstall ]; then
    say "Removing PrinterTUI"
    rm -f "$BIN"
    rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/printertui" "${TMPDIR:-/tmp}/printertui" "${TMPDIR:-/tmp}/printertui-scan"
    # installed from source by an earlier run
    if [ -f "$HOME/.local/.crates.toml" ] && command -v cargo >/dev/null; then
        cargo uninstall --root "$HOME/.local" printertui 2>/dev/null || true
    fi
    say "Done. CUPS, ImageMagick and your printers were left as they are."
    exit 0
fi

case "$(uname -s)" in
    Linux)
        command -v pacman >/dev/null || die "only Arch Linux and macOS are supported"
        say "Installing CUPS and ImageMagick (sudo may ask for your password)"
        sudo pacman -S --needed --noconfirm cups cups-filters imagemagick
        sudo systemctl enable --now cups.socket
        os=linux ;;
    Darwin)
        command -v brew >/dev/null || die "Homebrew is needed first: https://brew.sh"
        say "Installing qpdf and ImageMagick"
        brew install qpdf imagemagick
        os=macos ;;
    *) die "unsupported system: $(uname -s)" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) arch=$(uname -m) ;;
esac

mkdir -p "$BIN_DIR"
say "Downloading PrinterTUI ($os $arch)"
if curl -fsSL "https://github.com/$REPO/releases/latest/download/printertui-$os-$arch" -o "$BIN.tmp"; then
    chmod +x "$BIN.tmp"
    mv "$BIN.tmp" "$BIN"
elif command -v cargo >/dev/null; then
    rm -f "$BIN.tmp"
    say "No prebuilt binary for $os $arch, building from source"
    cargo install --git "https://github.com/$REPO" --root "$HOME/.local"
else
    rm -f "$BIN.tmp"
    die "no prebuilt binary for $os $arch; install Rust (https://rustup.rs) and run this again"
fi

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        case "$(basename "${SHELL:-sh}")" in
            zsh) rc="$HOME/.zshrc" ;;
            bash) rc="$HOME/.bashrc" ;;
            *) rc="" ;;
        esac
        if [ -n "$rc" ]; then
            echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$rc"
            say "Added ~/.local/bin to your PATH in $rc, open a new terminal"
        else
            say "Add ~/.local/bin to your PATH"
        fi ;;
esac

say "Installed. Run: printertui"
say "Optional: LibreOffice for non-PDF files, SANE for USB scanners"
