#!/bin/sh
# Try printertui without paper or a scanner, from a checkout:  sh test/printer-debug.sh [file]
#   Scanner: "Fake scanner" scans test/scan-page.typ, an article page on the wave equation (needs typst).
#   Printer: pick "PDF", which writes what you print to ~/PDF instead of paper
#            (`bash test/dev-setup-arch.sh fake` adds it on Arch).
set -eu
cd "$(dirname "$0")/.."
page=${TMPDIR:-/tmp}/printertui-scan-page.png
typst compile --ppi 300 test/scan-page.typ "$page"
lpstat -v PDF >/dev/null 2>&1 || echo "No PDF printer yet: bash test/dev-setup-arch.sh fake" >&2
cargo build --release
PRINTERTUI_FAKE_SCAN=$page exec target/release/printertui "$@"
