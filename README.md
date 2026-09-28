# PrinterTUI

Terminal UI for Linux, macOS and Windows to print and scan (CUPS and AirScan or SANE; the print spooler and the Windows scanner API on Windows), with manual double-sided printing for printers without a duplexer.

[![PrinterTUI demo](docs/demo.gif)](docs/demo.mp4)

Recorded in Ghostty; [watch it as a video, with the printer's sounds](docs/demo.mp4).

## Install

Arch Linux or macOS, one command and `printertui` is ready to run:

```sh
curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh
```

It asks for a language (your system's comes first, and the app then speaks it too), offers the optional extras and checks the download against its SHA-256 before installing it to `/usr/local/bin` (Homebrew's `bin` when you have it), which is already on your PATH. Run it again to Update or Uninstall, or uninstall directly:

```sh
curl -fsSL https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.sh | sh -s uninstall
```

Windows 10/11, in PowerShell (no administrator needed; puts `printertui` on your PATH and in the Start menu, run it again to update or uninstall):

```powershell
irm https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.ps1 | iex
```

**Nothing else to install on macOS**: printing goes through the CUPS that macOS has, pages are drawn for the preview by the system's PDFKit, searchable PDFs use the system's text recognition (Vision), and files are picked in the system's file dialog. Homebrew is only needed for the optional extras.

On Windows everything is built in too: PDFs are drawn for the printer by an embedded pdfium, and searchable PDFs use Windows' own text recognition (add a language with "Optical character recognition" in Settings if it says none is installed).

PDFs, photos (JPEG, PNG) and text files print everywhere without anything else; LibreOffice is only for other documents (Word, spreadsheets, slides...).

Development setup on Arch (dependencies, CUPS, build, link the build as `printertui-dev` so an installed `printertui` is left alone):

```sh
bash test/dev-setup-arch.sh
```

`sh test/installer-preview.sh [fresh|jam|smudge]` plays the installer's menus and animations without changing anything.

Optional: LibreOffice for documents (`libreoffice-fresh` / `brew install --cask libreoffice`), and SANE for scanners that are not part of a network printer (`sane sane-airscan` / `brew install sane-backends`). Network printers that can scan are used directly over AirScan (eSCL). On Linux, searchable PDFs (OCR) need Tesseract with the languages you scan (`tesseract tesseract-data-eng tesseract-data-spa`).

## Use

```sh
printertui [file]
```

| Key | Action |
| --- | --- |
| Tab | Switch between Print, Scan and Settings |
| `j` / `k`, `gg` / `G` | Move |
| `h` / `l` | Change option |
| `H` / `L` | Browse the preview's pages, or the scanned pages |
| `i` | Type in File or Pages (Esc to finish) |
| Enter | Pick files, pick pages, print, scan, save |
| `q` | Quit |

**Preview.** The page about to print is shown on the right as it will come out, converted and scaled: `H`/`L` browse the pages, or the sheets with several pages per sheet. Images show in full quality in kitty and Ghostty (also inside tmux with `set -g allow-passthrough on`) and in Sixel terminals (Windows Terminal, foot, WezTerm, mintty), in colored blocks elsewhere.

**Scale** shrinks or enlarges the content (25-200%) on the same paper, centered; above 100% the edges are cut off. **Photos** (JPEG, PNG) print without LibreOffice, filling the paper and turned upright as the camera took them.

**The printer.** The pixel-art printer from the installer lives in the app: it prints the pages you send, scans with a sweeping light, hops when you change a setting and jams on errors. It also tells how the real printer is doing, asked over IPP every 20 seconds: its ink tanks fill up to the real levels (in red when low), its tray empties when the printer is out of paper, its lights go out when the printer is off or paused, and it says what to do about a jam or an open cover. On Windows the problems come from the print spooler and the ink over IPP, for printers added by IP address.

**Double-sided**: the front pages print, then an animation shows how to flip the stack and put it back; press Enter for the back pages. If they come out in the wrong order, set **Back order** to *Reversed*.

**Scan**: put a page on the glass, press Enter on **Scan page**, repeat for more pages, then **Save** them as one PDF (optionally searchable) or as PNG files, or **Copy** them: they print at their real size with the Print tab's settings (with nothing scanned yet, Copy scans a page and prints it, like a photocopier). On the **Page** row: `h`/`l` browse pages (`H`/`L` from any row), `r`/`R` rotate, `f` filter (gray, black & white), `x` keep or leave out, `<`/`>` reorder, `dd` delete.

**Settings** (the third tab): the language, the theme (Auto keeps your terminal's colors, or Dark and Light), the volume of the printer's sounds (it whirrs when it prints, hums when it scans, grumbles when it jams; 0 mutes it), the mascot, how pages are drawn (Auto, kitty, Sixel or colored blocks) and the folder scans are saved in. Changes are saved to the settings file right away, and the tab shows it as it is on disk.

**Languages**: English, 中文, हिन्दी, Español, العربية, Français, বাংলা, Português, Русский and Bahasa Indonesia, the same as the installer. The app follows the language picked in the installer's menu, else the system's; change it any time in Settings.

Settings are remembered in `~/.config/printertui/config` (`%APPDATA%\printertui\config` on Windows).
