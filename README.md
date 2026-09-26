# PrinterTUI

A minimal, monochrome terminal UI for printing on Linux through CUPS.

## Features

- Pick a printer, color or grayscale, paper size and page range (`1-3,7,9-`).
- Manual double-sided printing for printers without a duplexer: the front pages
  are sent first, then you flip the stack and press Enter to print the back pages.
  If the back side comes out in the wrong order, set **Back order** to *Reversed*.
- Add network printers (driverless IPP Everywhere) from inside the app.

## Requirements

`cups`, `cups-filters` (provides `qpdf`, used to count PDF pages) and `sudo` for adding printers.

```sh
sudo pacman -S cups cups-filters
sudo systemctl enable --now cups.socket
```

## Usage

```sh
cargo run --release -- ~/document.pdf
```

| Key | Action |
| --- | --- |
| Up / Down / Tab, `j` / `k` | Move between fields |
| Left / Right, `h` / `l` | Change option |
| `i` / `a` | Edit the file path or page range (insert mode, Esc or Enter to finish) |
| Enter | On File: open a file picker (yazi, lf, ranger, nnn or fzf, first one installed). Elsewhere: print, or open the selected button |
| `q` / Esc / Ctrl+C | Quit |

Double-sided printing requires a PDF file.
