# PrinterTUI

Terminal UI for Linux and macOS to print through CUPS and scan over AirScan or SANE, with manual double-sided printing for printers without a duplexer.

![PrinterTUI demo](docs/demo.gif)

## Install

Arch Linux or macOS (with [Homebrew](https://brew.sh)):

```sh
curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh
```

Uninstall:

```sh
curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh -s uninstall
```

Optional: LibreOffice for non-PDF files (`libreoffice-fresh` / `brew install --cask libreoffice`), and SANE for scanners that are not part of a network printer (`sane sane-airscan` / `brew install sane-backends`). Network printers that can scan are used directly over AirScan (eSCL).

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
