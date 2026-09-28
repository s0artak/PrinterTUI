# PrinterTUI

Terminal UI for Linux, macOS and Windows to print and scan (CUPS and AirScan or SANE; the print spooler and the Windows scanner API on Windows), with manual double-sided printing for printers without a duplexer.

![PrinterTUI demo](docs/demo.gif)

## Install

Arch Linux or macOS (with [Homebrew](https://brew.sh)), one command and `printertui` is ready to run:

```sh
curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh
```

It asks for a language, offers the optional extras (LibreOffice, SANE, Tesseract) and checks the download against its SHA-256 before installing it to `/usr/local/bin` (`$(brew --prefix)/bin` on macOS), which is already on your PATH. Run it again to Update or Uninstall, or uninstall directly:

```sh
curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh -s uninstall
```

Windows 10/11, in PowerShell (no administrator needed; puts `printertui` on your PATH and in the Start menu, run it again to update or uninstall):

```powershell
irm https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.ps1 | iex
```

On Windows everything is built in: PDFs are drawn for the printer by an embedded pdfium, and searchable PDFs use Windows' own text recognition (add a language with "Optical character recognition" in Settings if it says none is installed). LibreOffice is optional, for non-PDF files.

Development setup on Arch (dependencies, CUPS, build, link the build as `printertui-dev` so an installed `printertui` is left alone):

```sh
bash test/dev-setup-arch.sh
```

`sh test/installer-preview.sh [fresh|jam|smudge]` plays the installer's menus and animations without changing anything.

Optional: LibreOffice for non-PDF files (`libreoffice-fresh` / `brew install --cask libreoffice`), and SANE for scanners that are not part of a network printer (`sane sane-airscan` / `brew install sane-backends`). Network printers that can scan are used directly over AirScan (eSCL). For searchable PDFs (OCR) install Tesseract with the languages you scan (`tesseract tesseract-data-eng tesseract-data-spa` / `brew install tesseract tesseract-lang`).

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

The page about to print is previewed on the right, converted and scaled as it will come out: `H`/`L` browse the pages (or the sheets, with several pages per sheet). **Scale** shrinks or enlarges the content (25-200%) on the same paper. Photos (JPEG, PNG) print without LibreOffice, filling the paper and turned upright. The printer's ink levels show under the form when it reports them (CUPS). The pixel-art printer from the installer keeps you company: it prints, scans and jams along with the app.

Double-sided: the front pages print, then flip the stack and press Enter. An animation shows how to flip it. If the back pages come out in the wrong order, set **Back order** to *Reversed*.

Scan: put a page on the glass, press Enter on **Scan page**, repeat for more pages, then **Save** them as one PDF or as PNG files, or **Copy** them: they print at their real size with the Print tab's settings (with nothing scanned yet, Copy scans a page and prints it, like a photocopier). On the **Page** row: `h`/`l` browse pages, `r`/`R` rotate, `f` filter (gray, black & white), `x` keep or leave out, `H`/`L` reorder, `dd` delete. The last scanned page is previewed on the right: as the real image in kitty and Ghostty (also inside tmux with `set -g allow-passthrough on`), in grayscale blocks elsewhere.

Settings are remembered in `~/.config/printertui/config`.
