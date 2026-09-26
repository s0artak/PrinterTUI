# PrinterTUI

A minimal, monochrome terminal UI for printing on Linux through CUPS.

## Features

- Pick a printer, color or grayscale, paper size, page range (`1-3,7,9-`),
  number of copies (collated) and pages per sheet (1, 2, 4, 6, 9 or 16).
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
  With several pages per sheet, whole sheet sides are split between the front and
  back passes. With several copies and an odd number of sheet sides, the copies come
  out uncollated (so the sheets without a back side are all at the top of the stack).
- Remembers the last settings that printed successfully (printer, color, sides,
  back order, paper, copies, pages per sheet, not the file or page range) in
  `$XDG_CONFIG_HOME/printertui/config` (default `~/.config/printertui/config`),
  as plain `key=value` lines. If the saved printer is gone, the default printer is used.
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
| Left / Right, `h` / `l` | Change option (Copies: decrease / increase) |
| `i` / `a` | Edit the file paths (separated by `;`) or page range (insert mode, Esc or Enter to finish) |
| Enter | On File: open a file picker (yazi, lf, ranger, nnn or fzf, first one installed; multiple selection works). Elsewhere: print, or open the selected button |
| `q` / Esc / Ctrl+C | Quit |

File names containing `;` are not supported.
