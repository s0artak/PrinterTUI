#!/bin/sh
# Prints the Scoop manifest of a release, from its Windows binaries' checksums (the release job
# attaches it to the release):
#   sh packaging/scoop.sh 0.4.0 s0artak/PrinterTUI dist > printertui.json
# People then install, and update, with:
#   scoop install https://github.com/s0artak/PrinterTUI/releases/latest/download/printertui.json
set -eu
version=$1 repo=$2 dir=$3
hash() { cut -d' ' -f1 "$dir/printertui-windows-$1.exe.sha256"; }
release="https://github.com/$repo/releases/download/v"
cat <<EOF
{
    "version": "$version",
    "description": "Print and scan from the terminal: preview, double-sided, network scanners, OCR",
    "homepage": "https://github.com/$repo",
    "license": "MIT",
    "architecture": {
        "64bit": {
            "url": "$release$version/printertui-windows-x86_64.exe#/printertui.exe",
            "hash": "$(hash x86_64)"
        },
        "arm64": {
            "url": "$release$version/printertui-windows-aarch64.exe#/printertui.exe",
            "hash": "$(hash aarch64)"
        }
    },
    "bin": "printertui.exe",
    "checkver": "github",
    "autoupdate": {
        "architecture": {
            "64bit": { "url": "$release\$version/printertui-windows-x86_64.exe#/printertui.exe" },
            "arm64": { "url": "$release\$version/printertui-windows-aarch64.exe#/printertui.exe" }
        },
        "hash": { "url": "\$url.sha256" }
    }
}
EOF
