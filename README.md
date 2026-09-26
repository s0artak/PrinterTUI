# PrinterTUI

Terminal UI for printing through CUPS and scanning through SANE, with manual double-sided printing for printers without a duplexer.

## Install

```sh
sudo pacman -S cups cups-filters libreoffice-fresh sane sane-airscan imagemagick
sudo systemctl enable --now cups.socket
cargo install --path .
```

LibreOffice is only needed for non-PDF files, SANE and ImageMagick only for scanning.

## Use

```sh
printertui [file]
```

| Key | Action |
| --- | --- |
| Tab | Switch between Print and Scan |
| `j` / `k`, `gg` / `G` | Move |
| `h` / `l` | Change option |
| `i` | Type in File or Pages (Esc to finish) |
| Enter | Pick files, pick pages, print, scan, save |
| `q` | Quit |

Double-sided: the front pages print, then flip the stack and press Enter. An animation shows how to flip it. If the back pages come out in the wrong order, set **Back order** to *Reversed*.

Scan: put a page on the glass, press Enter on **Scan page**, repeat for more pages, then **Save** them as one PDF or as PNG files. The last scanned page is previewed on the right.

Settings are remembered in `~/.config/printertui/config`.
