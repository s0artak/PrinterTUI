#!/bin/sh
# Plays the installer's menus and animations without installing, removing or downloading anything.
#   sh test/installer-preview.sh           as on a machine that has PrinterTUI: Update / Uninstall
#   sh test/installer-preview.sh fresh     as on a first install: Install
#   sh test/installer-preview.sh jam       first install where the download fails: paper jam
#   sh test/installer-preview.sh smudge    first install where the checksum does not match
cd "$(dirname "$0")/.."
PRINTERTUI_PREVIEW=${1:-installed} exec sh install.sh
