# PrinterTUI

Terminal UI for printing through CUPS, with manual double-sided printing for printers without a duplexer.

## Install

```sh
sudo pacman -S cups cups-filters libreoffice-fresh
sudo systemctl enable --now cups.socket
cargo install --path .
```

LibreOffice is only needed for non-PDF files.

## Use

```sh
printertui [file]
```

| Key | Action |
| --- | --- |
| `j` / `k`, `gg` / `G` | Move |
| `h` / `l` | Change option |
| `i` | Type in File or Pages (Esc to finish) |
| Enter | Pick files, pick pages, print |
| `q` | Quit |

Double-sided: the front pages print, then flip the stack and press Enter. Press `v` for a video of how to flip it. If the back pages come out in the wrong order, set **Back order** to *Reversed*.

Settings are remembered in `~/.config/printertui/config`.
