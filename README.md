# PrinterTUI

A minimal, monochrome terminal UI for printing on Linux through CUPS.

## Features

- Pick a printer, color or grayscale, paper size and page range (`1-3,7,9-`).
- Print any file: anything that is not a PDF (office documents, text, images) is
  converted to PDF with LibreOffice first, so page ranges and duplex work for every format.
- Print several files at once: select many in the file picker, or type paths separated
  by `;` (`a.pdf; ~/b.docx`). The page range applies to each file.
- Manual double-sided printing for printers without a duplexer: the front pages
  are sent first, and once the printer has finished them (the app polls the CUPS queue)
  you flip the stack and press Enter to print the back pages. With several files this
  repeats for each file in turn.
  If the back side comes out in the wrong order, set **Back order** to *Reversed*.
  Press `v` in the duplex prompt to watch a short video showing how to flip the paper
  (rendered with Blender from `assets/duplex.py`).
- Add network printers (driverless IPP Everywhere) from inside the app.

## Requirements

`cups`, `cups-filters` (provides `qpdf`, used to count PDF pages), `libreoffice`
(only for non-PDF files) and `sudo` for adding printers.

```sh
sudo pacman -S cups cups-filters libreoffice-fresh
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
| `i` / `a` | Edit the file paths (separated by `;`) or page range (insert mode, Esc or Enter to finish) |
| Enter | On File: open a file picker (yazi, lf, ranger, nnn or fzf, first one installed; multiple selection works). Elsewhere: print, or open the selected button |
| `q` / Esc / Ctrl+C | Quit |

File names containing `;` are not supported.
