# PrinterTUI on Windows

Goal: one `printertui.exe` that prints, scans and saves on Windows 10/11 with nothing else to install
(LibreOffice stays optional for Office files, as on Linux/macOS). It stays the same terminal UI:
crossterm runs in Windows Terminal, which Windows 11 opens on double-click.

## Decisions

| Area | Linux / macOS today | Windows | Why |
| --- | --- | --- | --- |
| Printers, default, jobs, cancel | `lpstat`, `lpoptions`, `lpq`, `cancel` | winspool: `EnumPrintersW`, `GetDefaultPrinterW`, `EnumJobsW`, `SetJobW` | Built into Windows, no admin needed |
| Print a PDF | `lp` | pdfium (`pdfium-render`) draws each page into a GDI printer DC, `StartDocW` gives the job id | Permissive license, `pdfium.dll` embedded in the exe and unpacked on first use: still one file. SumatraPDF was the alternative, but it is GPL and a second exe |
| Page ranges, copies, manual duplex | `lp -o page-ranges ...` | Same `parse_ranges`/`split_duplex`, pages picked in Rust, copies in `DEVMODE` | The logic is already pure Rust |
| Find and add network printers | `lpinfo -v`, `sudo lpadmin -m everywhere` | mDNS `_ipp._tcp` (`mdns-sd`), `Add-Printer -IppURL` in an elevated PowerShell | Windows picks its IPP class driver, the `-m everywhere` equivalent |
| Scan | eSCL over `curl`, SANE `scanimage` | eSCL over `minreq` + WinRT `ImageScanner` (USB, and network scanners Windows already added) | WinRT covers WIA without COM plumbing |
| Page count | `qpdf` | `lopdf` | Pure Rust |
| Rotate, filters, preview | ImageMagick `magick` | `image` crate | Pure Rust |
| Scans to PDF | `magick` | `pdf-writer` | Pure Rust, JPEGs go in as they are |
| OCR (searchable PDF) | `tesseract` | `Windows.Media.Ocr` + invisible text layer via `pdf-writer`; `tesseract.exe` if on PATH | No install; greyed out when no OCR language |
| Office files | LibreOffice | LibreOffice at `Program Files\LibreOffice\program\soffice.exe` | Optional, as today |
| File picker | yazi/lf/ranger/nnn/fzf | Native dialog (`rfd`) | |
| Stop children on quit | `pkill`/`kill` | `taskkill /T /F` | |
| Settings | `~/.config/printertui/config` | `%APPDATA%\printertui\config` | |
| Image preview | kitty graphics or half blocks | half blocks | Sixel later, only if wanted |

The pure-Rust replacements (`image`, `pdf-writer`, `lopdf`, `minreq`) go in for Linux and macOS too:
one code path to test, and ImageMagick, qpdf and curl stop being dependencies there.

## Phases

1. **Seam.** `src/backend/{mod,unix,windows}.rs`, chosen with `#[cfg]`: the same free functions on both
   sides (printers, printer_labels, submit(&Job), job_active, queue, cancel_job, discover, add_printer,
   pick_files, scanners, scan, to_pdf, kill_children). Portable path helpers: config dir, home,
   timestamp without `date`, `\` in file names. Linux/macOS behave exactly as before.
2. **Pure-Rust documents and images** on every platform: `lopdf` page count, `image` for
   thumbnail/edit_page, `pdf-writer` for save_scans, `minreq` for eSCL. Tests for each.
3. **Windows printing:** winspool listing and jobs, pdfium printing, `Add-Printer` + mDNS.
4. **Windows scanning and OCR:** `ImageScanner`, `Windows.Media.Ocr` text layer.
5. **CI and install:** `windows-latest` (x86_64) and `windows-11-arm` (aarch64) jobs that build and run
   the tests, `.exe` + `.sha256` in releases, `install.ps1` for `irm ... | iex` with the same checksum check.
6. **Try it on real Windows** with a printer and a scanner (needs the owner): checklist below.

## Status

Phases 1–5 are done. CI builds Windows x86_64 and ARM and runs the tests there, including real
prints through pdfium to "Microsoft Print to PDF" that check how large the pages come out (one
to a sheet at their own size, two to a sideways sheet filling their halves), and photo decoding
with Windows' own codecs. `install.ps1` has the same menus, languages and pixel-art printer as
`install.sh`; `pwsh test/installer-preview.ps1 [fresh|jam|smudge]` plays it, and CI runs it
to check it leaves the user's PowerShell session as it was.

Fixed after the first review:
- Pages printed about 2 inches wide: pdfium drew them at 72 dpi and GDI shrank that further.
- Adding a printer found on the network could run code: its name reached PowerShell inside
  quotes that a typographic ’ could end. Names and addresses now only reach PowerShell as quoted
  literals in -EncodedCommand scripts, and a failed Add-Printer says why.
- Printing froze the screen while pages were drawn; it now runs in the background.
- Dots per inch across and down taken separately, landscape pages turned on upright paper,
  2 and 6 pages per sheet on a sideways sheet (as CUPS does), copies made by the driver, color
  asked for explicitly.
- HEIC, AVIF, JPEG XR and camera raw photos go through Windows' decoders (HEIC and AVIF need the
  free Microsoft Store extensions); BMP, GIF, TIFF and WebP are read by the app itself.
- Paths in quotes ("Copy as path", files dragged onto the terminal) are understood.
- Scans go to the real Documents folder (OneDrive when its backup is on).
- Ink levels for the printers the app adds; the file dialog opens in front of the terminal;
  Sixel images sized for Windows Terminal's 10 x 20 cells.

Known limits:
- Untested on real hardware: printing to a physical printer, scanning (WinRT), OCR, Add-Printer, the file dialog.
- Wine can start the exe but its console garbles full-screen apps, and it cannot run PowerShell,
  so neither is a stand-in for Windows.
- Flags in the language menu show as letters on Windows (no flag emoji in Windows' fonts).
- OCR uses the Windows user's language only, not several at once like tesseract.
- Ink levels for network printers Windows added by itself (a "WSD-..." port) are not shown.
- The exe is unsigned: SmartScreen warns on first run until it is signed or well known
  (see [windows-packaging.md](windows-packaging.md)).

Later, only if asked for: a window of its own instead of the console (below).

## Checklist on a real Windows machine

- [ ] Double-click `printertui.exe` opens it in Windows Terminal
- [ ] Printers listed, default one selected
- [ ] Print a PDF: page range, copies, manual double-sided, cancel a job
- [ ] Add a network printer
- [ ] Scan from a network scanner and a USB one
- [ ] Rotate, filter, reorder, save as PDF / PNG / searchable PDF
- [ ] Print a .docx with LibreOffice installed, clear error without it
- [ ] Settings remembered between runs

## A window of its own (proposal, not started)

The console is the weak part on Windows: Windows 10 opens the old console host on double-click
(its fonts lack some symbols, it has no image protocol), Windows Terminal only shows Sixel images
from version 1.22, and both redraw slowly. The app could open its own window there and keep
the same screens: ratatui draws into a Buffer, and backends exist that paint that buffer into a
window ([egui_ratatui](https://github.com/gold-silver-copper/egui_ratatui) on top of
soft_ratatui, [ratatui-wgpu](https://docs.rs/ratatui-wgpu)).

What is already in place: `App::on_key` takes crossterm key events and `App::mouse` mouse
events, and `ui::draw` draws a Frame, none of them tied to the terminal. A window build would:

1. Open a window with eframe and egui_ratatui (`--tui` keeps the console version), and turn
   egui's key and mouse events into the same crossterm events.
2. Draw the preview as a real image (an egui texture over the preview panel), instead of
   kitty/Sixel/half blocks.
3. Use Windows' own fonts for the grid (Cascadia Mono, with Microsoft YaHei, Nirmala UI and
   Segoe UI as fallbacks for Chinese, Hindi/Bengali and Arabic), so the exe does not grow by a
   CJK font.
4. Take files dropped onto the window as the File to print.

Cost: about a week to a usable version (resizing, high-DPI, clipboard, icon, remembering the
window size), and 3 to 8 MB more in the exe. The file dialog, Add-Printer and printing need no
change; the console's restore/init around them goes away.

