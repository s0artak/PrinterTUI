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

Phases 1–5 are done. CI builds Windows x86_64 and ARM and runs the tests there, including a real
print through pdfium to "Microsoft Print to PDF". `install.ps1` has the same menus, languages and
pixel-art printer as `install.sh`; `pwsh test/installer-preview.ps1 [fresh|jam|smudge]` plays it.

Known limits:
- Untested on real hardware: printing to a physical printer, scanning (WinRT), OCR, Add-Printer, the file dialog.
- Wine can start the exe but its console garbles full-screen apps, and it cannot run PowerShell,
  so neither is a stand-in for Windows.
- Flags in the language menu show as letters on Windows (no flag emoji in Windows' fonts).
- Pages per sheet are laid out in a grid without rotating them the way CUPS does.
- OCR uses the Windows user's language only, not several at once like tesseract.
- The exe is unsigned: SmartScreen warns on first run until it is signed or well known.
- The installer needs a release that has the Windows exe (a new tag after merging).

Later, only if asked for: code signing (SignPath, free for open source), winget/Scoop, a GUI (egui).

## Checklist on a real Windows machine

- [ ] Double-click `printertui.exe` opens it in Windows Terminal
- [ ] Printers listed, default one selected
- [ ] Print a PDF: page range, copies, manual double-sided, cancel a job
- [ ] Add a network printer
- [ ] Scan from a network scanner and a USB one
- [ ] Rotate, filter, reorder, save as PDF / PNG / searchable PDF
- [ ] Print a .docx with LibreOffice installed, clear error without it
- [ ] Settings remembered between runs
