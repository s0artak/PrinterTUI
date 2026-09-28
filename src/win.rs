//! Windows: the print spooler (winspool), pdfium to draw PDF pages on the printer, WinRT for
//! scanners and OCR, and PowerShell's Add-Printer for network printers.

use crate::*;
use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::DEVPROPTYPE;
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Devices::Scanners::{ImageScanner, ImageScannerColorMode, ImageScannerFormat, ImageScannerResolution, ImageScannerScanSource};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::StorageFolder;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Printing::*;
use windows::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows::Win32::System::SystemInformation::GetLocalTime;

pub const ADD_PRINTER_NOTE: &str = "Windows will ask you to allow it";
pub const PICK_HINT: &str = "the file dialog was closed";
pub const SCANNER_HINT: &str = "USB scanners need their Windows driver installed";

/// Printer of each job id seen, since winspool needs the printer to look a job up.
static JOB_PRINTER: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);

fn remember(id: u32, printer: &str) {
    JOB_PRINTER.lock().unwrap().get_or_insert_default().insert(id, printer.to_string());
}

fn printer_of(id: &str) -> Option<(u32, String)> {
    let id: u32 = id.parse().ok()?;
    Some((id, JOB_PRINTER.lock().unwrap().as_ref()?.get(&id)?.clone()))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn text(p: PWSTR) -> String {
    if p.is_null() { String::new() } else { unsafe { p.to_string() }.unwrap_or_default() }
}

/// Runs `f` with an open handle to `printer`.
fn with_printer<T>(printer: &str, f: impl FnOnce(PRINTER_HANDLE) -> T) -> Result<T, String> {
    let name = wide(printer);
    let mut h = PRINTER_HANDLE::default();
    unsafe { OpenPrinterW(PCWSTR(name.as_ptr()), &mut h, None) }.map_err(|e| format!("{printer}: {e}"))?;
    let out = f(h);
    let _ = unsafe { ClosePrinter(h) };
    Ok(out)
}

/// Calls a winspool "fill this buffer" function twice: once for the size, once for the data.
/// Returns the buffer (u64s, so the structs in it are aligned) and the number of entries.
fn enum_buffer(call: impl Fn(Option<&mut [u8]>, &mut u32, &mut u32) -> bool) -> (Vec<u64>, usize) {
    let (mut needed, mut count) = (0, 0);
    call(None, &mut needed, &mut count);
    if needed == 0 {
        return (Vec::new(), 0);
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, needed as usize) };
    if !call(Some(bytes), &mut needed, &mut count) {
        return (Vec::new(), 0);
    }
    (buf, count as usize)
}

/// Installed printers as (name, port, attributes).
fn installed() -> Vec<(String, String, u32)> {
    let (buf, n) = enum_buffer(|b, need, count| unsafe {
        EnumPrintersW(PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS, PCWSTR::null(), 2, b, need, count).is_ok()
    });
    let infos = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const PRINTER_INFO_2W, n) };
    infos.iter().map(|p| (text(p.pPrinterName), text(p.pPortName), p.Attributes)).collect()
}

/// Network address in a printer port: "IP_192.168.1.46", "192.168.1.46" or "http://host:631/...".
fn port_host(port: &str) -> Option<String> {
    if let Some((_, host)) = uri_host(port) {
        return Some(host.to_string());
    }
    let ip = port.strip_prefix("IP_").unwrap_or(port);
    ip.parse::<std::net::Ipv4Addr>().ok().map(|_| ip.to_string())
}

/// Installed printers, default printer first.
pub fn printers() -> Vec<String> {
    let mut list: Vec<String> = installed().into_iter().map(|(name, _, _)| name).collect();
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    if unsafe { GetDefaultPrinterW(Some(PWSTR(buf.as_mut_ptr())), &mut len) }.as_bool() {
        let def = String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize]);
        if let Some(i) = list.iter().position(|p| *p == def) {
            list.swap(0, i);
        }
    }
    list
}

/// The printer name, plus the network address for network printers.
pub fn printer_labels(queues: &[String]) -> Vec<String> {
    let all = installed();
    queues
        .iter()
        .map(|q| match all.iter().find(|(name, _, _)| name == q).and_then(|(_, port, _)| port_host(port)) {
            Some(host) => format!("{q} ({host})"),
            None => q.clone(),
        })
        .collect()
}

/// Hosts of the installed network printers, to try as eSCL scanners.
pub fn printer_hosts() -> Vec<String> {
    let mut hosts: Vec<String> = installed().iter().filter_map(|(_, port, _)| port_host(port)).collect();
    hosts.extend(ipp_urls().iter().filter_map(|u| uri_host(u)).map(|(_, host)| host.to_string()));
    hosts.sort();
    hosts.dedup();
    hosts
}

/// URLs of the printers Windows added over IPP (Add-Printer -IppURL, or found on the network):
/// their port is just "WSD-<id>", the URL is a property of their SWD\IPP\... device.
fn ipp_urls() -> Vec<String> {
    // the printer URI property, {A35996AB-11CF-4935-8B61-A6761081ECDF} 12 in Get-PnpDeviceProperty
    const URI: DEVPROPKEY = DEVPROPKEY { fmtid: windows::core::GUID::from_u128(0xa35996ab_11cf_4935_8b61_a6761081ecdf), pid: 12 };
    let filter = wide("SWD");
    let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
    let mut len = 0;
    if unsafe { CM_Get_Device_ID_List_SizeW(&mut len, PCWSTR(filter.as_ptr()), flags) } != CR_SUCCESS {
        return Vec::new();
    }
    let mut ids = vec![0u16; len as usize];
    if unsafe { CM_Get_Device_ID_ListW(PCWSTR(filter.as_ptr()), &mut ids, flags) } != CR_SUCCESS {
        return Vec::new();
    }
    ids.split(|&c| c == 0)
        .filter(|id| String::from_utf16_lossy(id).to_ascii_uppercase().starts_with("SWD\\IPP\\"))
        .filter_map(|id| {
            let id: Vec<u16> = id.iter().copied().chain([0]).collect();
            let mut node = 0;
            let mut buf = [0u16; 512];
            let mut size = (buf.len() * 2) as u32;
            let mut kind = DEVPROPTYPE::default();
            unsafe {
                if CM_Locate_DevNodeW(&mut node, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS
                    || CM_Get_DevNode_PropertyW(node, &URI, &mut kind, Some(buf.as_mut_ptr().cast()), &mut size, 0) != CR_SUCCESS
                {
                    return None;
                }
            }
            Some(String::from_utf16_lossy(&buf[..size as usize / 2]).trim_end_matches('\0').to_string())
        })
        .collect()
}

/// pdfium.dll, embedded at build time (see build.rs) and unpacked next to the settings on first use.
/// It can only be loaded once per process, so every print shares it.
fn pdfium() -> Result<&'static Pdfium, String> {
    static PDFIUM: std::sync::OnceLock<Pdfium> = std::sync::OnceLock::new();
    if let Some(p) = PDFIUM.get() {
        return Ok(p);
    }
    const DLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pdfium.dll"));
    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("printertui");
    let path = dir.join("pdfium-7881.dll");
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(DLL.len() as u64) {
        std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, DLL)).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let bindings = Pdfium::bind_to_library(&path).map_err(|e| format!("pdfium: {e}"))?;
    Ok(PDFIUM.get_or_init(|| Pdfium::new(bindings)))
}

/// Printer settings for the job: paper, color, and always one-sided (double-sided is done by hand).
fn devmode(printer: &str, job: &Job) -> Result<Vec<u64>, String> {
    let name = wide(printer);
    with_printer(printer, |h| unsafe {
        let size = DocumentPropertiesW(None, h, PCWSTR(name.as_ptr()), None, None, 0);
        if size <= 0 {
            return Err(format!("{printer}: cannot read its settings"));
        }
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let dm = buf.as_mut_ptr() as *mut DEVMODEW;
        DocumentPropertiesW(None, h, PCWSTR(name.as_ptr()), Some(dm), None, DM_OUT_BUFFER.0);
        let paper = match job.paper {
            "Letter" => DMPAPER_LETTER,
            "Legal" => DMPAPER_LEGAL,
            "A5" => DMPAPER_A5,
            "A3" => DMPAPER_A3,
            _ => DMPAPER_A4,
        };
        (*dm).Anonymous1.Anonymous1.dmPaperSize = paper as i16;
        (*dm).dmDuplex = DMDUP_SIMPLEX;
        (*dm).dmFields |= DM_PAPERSIZE | DM_DUPLEX;
        if !job.color {
            (*dm).dmColor = DMCOLOR_MONOCHROME;
            (*dm).dmFields |= DM_COLOR;
        }
        // let the driver check the changes against what the printer can do
        DocumentPropertiesW(None, h, PCWSTR(name.as_ptr()), Some(dm), Some(dm), DM_IN_BUFFER.0 | DM_OUT_BUFFER.0);
        Ok(buf)
    })?
}

/// Draws the job's pages onto the printer with pdfium and GDI; returns the spooler job id.
/// `output` sends it to a file instead of the device (for printers like "Microsoft Print to PDF").
fn print_pdf(job: &Job, output: Option<&str>) -> Result<u32, String> {
    let pdfium = pdfium()?;
    let doc = pdfium.load_pdf_from_file(&job.file, None).map_err(|e| format!("{}: {e}", job.file))?;
    let total = doc.pages().len() as u32;
    let pages = parse_ranges(job.pages.as_deref().unwrap_or(""), total)?;
    let mut sheets: Vec<&[u32]> = pages.chunks(job.per_sheet as usize).collect();
    if job.reverse {
        sheets.reverse();
    }
    let copies = job.copies.max(1) as usize;
    let order: Vec<&[u32]> = if job.collate {
        (0..copies).flat_map(|_| sheets.iter().copied()).collect()
    } else {
        sheets.iter().flat_map(|s| std::iter::repeat_n(*s, copies)).collect()
    };

    let dm = devmode(&job.printer, job)?;
    let name = wide(&job.printer);
    let hdc = unsafe { CreateDCW(windows::core::w!("WINSPOOL"), PCWSTR(name.as_ptr()), PCWSTR::null(), Some(dm.as_ptr() as *const DEVMODEW)) };
    if hdc.is_invalid() {
        return Err(format!("{}: cannot open the printer", job.printer));
    }
    let doc_name = wide(std::path::Path::new(&job.file).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()).as_str());
    let output = output.map(wide);
    let info = DOCINFOW {
        cbSize: size_of::<DOCINFOW>() as i32,
        lpszDocName: PCWSTR(doc_name.as_ptr()),
        lpszOutput: output.as_ref().map_or(PCWSTR::null(), |o| PCWSTR(o.as_ptr())),
        ..Default::default()
    };
    let id = unsafe { StartDocW(hdc, &info) };
    let res = if id <= 0 {
        Err(format!("{}: the printer refused the job", job.printer))
    } else {
        remember(id as u32, &job.printer);
        let res = order.iter().try_for_each(|sheet| draw_sheet(hdc, &doc, sheet, job));
        unsafe {
            if res.is_ok() { EndDoc(hdc) } else { AbortDoc(hdc) };
        }
        res.map(|_| id as u32)
    };
    let _ = unsafe { DeleteDC(hdc) };
    res
}

/// One sheet side: its pages in a grid, each rendered at up to 300 dpi and scaled to its cell.
fn draw_sheet(hdc: HDC, doc: &PdfDocument, sheet: &[u32], job: &Job) -> Result<(), String> {
    let cap = |i| unsafe { GetDeviceCaps(Some(hdc), i) };
    let (width, height, dpi) = (cap(HORZRES), cap(VERTRES), cap(LOGPIXELSX).max(72));
    let (cols, rows) = grid(job.per_sheet);
    let (cols, rows) = (cols as i32, rows as i32);
    let (cell_w, cell_h) = (width / cols, height / rows);
    // rendering at the printer's full resolution would need hundreds of MB per page
    let scale = (300.0 / dpi as f32).min(1.0);
    if unsafe { StartPage(hdc) } <= 0 {
        return Err("The printer stopped accepting pages".into());
    }
    unsafe { SetStretchBltMode(hdc, HALFTONE) };
    for (k, &n) in sheet.iter().enumerate() {
        let page = doc.pages().get((n - 1) as PdfPageIndex).map_err(|e| format!("page {n}: {e}"))?;
        let config = PdfRenderConfig::new()
            .set_maximum_width((cell_w as f32 * scale) as Pixels)
            .set_maximum_height((cell_h as f32 * scale) as Pixels)
            .set_format(PdfBitmapFormat::BGRA)
            .use_grayscale_rendering(!job.color)
            .render_form_data(true);
        let bitmap = page.render_with_config(&config).map_err(|e| format!("page {n}: {e}"))?;
        let (bw, bh) = (bitmap.width(), bitmap.height());
        let (dw, dh) = ((bw as f32 / scale) as i32, (bh as f32 / scale) as i32);
        let (col, row) = (k as i32 % cols, k as i32 / cols);
        let (x, y) = (col * cell_w + (cell_w - dw) / 2, row * cell_h + (cell_h - dh) / 2);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bw,
                biHeight: -bh, // top-down rows, as pdfium writes them
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let pixels = bitmap.as_raw_bytes();
        unsafe { StretchDIBits(hdc, x, y, dw, dh, 0, 0, bw, bh, Some(pixels.as_ptr().cast()), &bmi, DIB_RGB_COLORS, SRCCOPY) };
    }
    if unsafe { EndPage(hdc) } <= 0 {
        return Err("The printer stopped accepting pages".into());
    }
    Ok(())
}

/// Ink levels are not read on Windows yet.
// ponytail: the spooler has no ink API; ask the printer over IPP (marker-levels) like CUPS does if wanted
pub fn ink(_queue: &str) -> Vec<(u32, i32)> {
    Vec::new()
}

/// One page of a PDF as a PNG for the preview, drawn by pdfium.
pub fn render_page(pdf: &str, page: u32, png: &str) -> Result<(), String> {
    let doc = pdfium()?.load_pdf_from_file(pdf, None).map_err(|e| format!("{pdf}: {e}"))?;
    let err = |e: PdfiumError| format!("page {page}: {e}");
    // the bitmap borrows the page, so it has to outlive it
    let pdf_page = doc.pages().get((page - 1) as PdfPageIndex).map_err(err)?;
    let bitmap = pdf_page.render_with_config(&PdfRenderConfig::new().set_target_width(1000).render_form_data(true)).map_err(err)?;
    let img = image::RgbaImage::from_raw(bitmap.width() as u32, bitmap.height() as u32, bitmap.as_rgba_bytes()).ok_or("pdfium: bad bitmap")?;
    img.save_with_format(png, image::ImageFormat::Png).map_err(|e| format!("{png}: {e}"))
}

/// Prints a job, returns "request id is <id> (<printer>)" like `lp` does.
pub fn submit(job: &Job) -> Result<String, String> {
    let id = print_pdf(job, None)?;
    JOBS.lock().unwrap().push(id.to_string());
    Ok(format!("request id is {id} ({})", job.printer))
}

/// True while the job is still in the printer's queue.
pub fn job_active(id: &str) -> bool {
    let Some((id, printer)) = printer_of(id) else { return false };
    with_printer(&printer, |h| {
        let mut needed = 0;
        // with no buffer the call fails either way, but it reports a size only for a job that exists
        let _ = unsafe { GetJobW(h, id, 1, None, &mut needed) };
        needed > 0
    })
    .unwrap_or(false)
}

/// Unfinished jobs on all printers as (job id, "file  (owner, printer, pages)") for the queue popup.
pub fn queue() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (printer, _, _) in installed() {
        let _ = with_printer(&printer, |h| {
            let (buf, n) = enum_buffer(|b, need, count| unsafe { EnumJobsW(h, 0, 999, 1, b, need, count).is_ok() });
            let jobs = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const JOB_INFO_1W, n) };
            for j in jobs {
                remember(j.JobId, &printer);
                let pages = if j.TotalPages > 0 { format!(", {} pages", j.TotalPages) } else { String::new() };
                out.push((j.JobId.to_string(), format!("{}  ({}, {printer}{pages})", text(j.pDocument), text(j.pUserName))));
            }
        });
    }
    out
}

pub fn cancel_job(id: &str) -> Result<(), String> {
    let (id, printer) = printer_of(id).ok_or(format!("Unknown job {id}"))?;
    let ok = with_printer(&printer, |h| unsafe { SetJobW(h, id, 0, None, JOB_CONTROL_DELETE) }.as_bool())?;
    if ok { Ok(()) } else { Err(format!("Could not cancel job {id}")) }
}

/// IPP printers announced on the network (mDNS `_ipp._tcp`) that are not installed yet,
/// as (name, IPP uri) for Add-Printer.
pub fn discover() -> Vec<(String, String)> {
    let Ok(mdns) = mdns_sd::ServiceDaemon::new() else { return Vec::new() };
    let Ok(events) = mdns.browse("_ipp._tcp.local.") else { return Vec::new() };
    let known = printer_hosts();
    let mut found = Vec::new();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while let Ok(event) = events.recv_deadline(end) {
        let mdns_sd::ServiceEvent::ServiceResolved(s) = event else { continue };
        let Some(ip) = s.get_addresses_v4().into_iter().next() else { continue };
        if known.contains(&ip.to_string()) {
            continue;
        }
        let path = s.get_property_val_str("rp").unwrap_or("ipp/print");
        let model = s.get_property_val_str("ty").map(str::to_string);
        let name = model.unwrap_or_else(|| s.fullname.split("._ipp").next().unwrap_or(&s.fullname).to_string());
        found.push((name, format!("ipp://{ip}:{}/{path}", s.get_port())));
    }
    let _ = mdns.shutdown();
    found.sort();
    found.dedup();
    found
}

/// Adds an IPP printer with Windows' own IPP driver; Windows asks for administrator rights.
pub fn add_printer(name: &str, uri: &str) -> Result<(), String> {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let add = format!("Add-Printer -Name {} -IppURL {}", quote(name), quote(uri));
    let elevate = format!(
        "$p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-Command',{}; exit $p.ExitCode",
        quote(&add)
    );
    run("powershell", &["-NoProfile", "-Command", &elevate])?;
    if printers().iter().any(|p| p == name) { Ok(()) } else { Err(format!("Windows did not add {name}")) }
}

/// The Windows file dialog.
pub fn pick_files() -> Vec<String> {
    rfd::FileDialog::new()
        .set_title("Files to print")
        .pick_files()
        .unwrap_or_default()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Scanners Windows knows (USB, and network scanners it added itself) as ("wia:<id>", name).
pub fn local_scanners() -> Vec<(String, String)> {
    let find = || -> windows::core::Result<Vec<(String, String)>> {
        let all = DeviceInformation::FindAllAsyncAqsFilter(&ImageScanner::GetDeviceSelector()?)?.join()?;
        Ok(all.into_iter().filter_map(|d| Some((format!("wia:{}", d.Id().ok()?), d.Name().ok()?.to_string()))).collect())
    };
    find().unwrap_or_default()
}

/// Scans one page from the flatbed with Windows' scanner API into `out`.
pub fn scan_local(device: &str, mode: &str, dpi: u32, out: &str) -> Result<(), String> {
    let id = device.strip_prefix("wia:").ok_or(format!("Unknown scanner {device}"))?;
    let dir = std::env::temp_dir().join(format!("printertui-wia-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let scan = || -> windows::core::Result<String> {
        let scanner = ImageScanner::FromIdAsync(&HSTRING::from(id))?.join()?;
        let flatbed = scanner.FlatbedConfiguration()?;
        flatbed.SetColorMode(if mode == "Gray" { ImageScannerColorMode::Grayscale } else { ImageScannerColorMode::Color })?;
        flatbed.SetDesiredResolution(ImageScannerResolution { DpiX: dpi as f32, DpiY: dpi as f32 })?;
        let format = if flatbed.IsFormatSupported(ImageScannerFormat::Png)? { ImageScannerFormat::Png } else { ImageScannerFormat::Jpeg };
        flatbed.SetFormat(format)?;
        let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(dir.as_os_str()))?.join()?;
        let result = scanner.ScanFilesToFolderAsync(ImageScannerScanSource::Flatbed, &folder)?.join()?;
        Ok(result.ScannedFiles()?.GetAt(0)?.Path()?.to_string())
    };
    // Windows picks the file name; move the page where the caller wants it
    let res = scan().map_err(|e| format!("Scan failed: {}", e.message())).and_then(|f| std::fs::rename(&f, out).map_err(|e| e.to_string()));
    let _ = std::fs::remove_dir_all(&dir);
    res
}

/// Stops running tools and their children.
pub fn kill_tree(pids: &[u32]) {
    for pid in pids {
        let _ = std::process::Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output();
    }
}

/// LibreOffice's soffice.exe in Program Files, else whatever is on the PATH.
pub fn soffice() -> String {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|d| std::path::Path::new(&d).join(r"LibreOffice\program\soffice.exe"))
        .find(|p| p.is_file())
        .map_or("soffice".into(), |p| p.to_string_lossy().into_owned())
}

/// Local time for file names: 2026-09-26_154200.
pub fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{}-{:02}-{:02}_{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// Searchable PDF with Windows' built-in OCR in the user's language, laid invisibly over the pages.
pub fn ocr_pdf(pages: &[String], out: &str, dpi: u32) -> Result<Vec<String>, String> {
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
        "Searchable PDF needs a Windows OCR language: Settings > Time & language > Language, add your language with 'Optical character recognition'".to_string()
    })?;
    let words = pages.iter().map(|p| ocr_page(&engine, p)).collect::<Result<Vec<_>, _>>()?;
    let path = format!("{out}.pdf");
    std::fs::write(&path, images_to_pdf(pages, dpi, &words)?).map_err(|e| format!("{path}: {e}"))?;
    Ok(vec![path])
}

fn ocr_page(engine: &OcrEngine, page: &str) -> Result<Vec<Word>, String> {
    let img = image::ImageReader::open(page)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?
        .to_luma8();
    // the engine has a size limit; shrink big scans and scale the boxes back
    let max = OcrEngine::MaxImageDimension().unwrap_or(10000).max(1);
    let scale = (max as f32 / img.width().max(img.height()) as f32).min(1.0);
    let img = if scale < 1.0 {
        image::imageops::resize(&img, (img.width() as f32 * scale) as u32, (img.height() as f32 * scale) as u32, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let recognise = || -> windows::core::Result<Vec<Word>> {
        let buffer = CryptographicBuffer::CreateFromByteArray(img.as_raw())?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Gray8, img.width() as i32, img.height() as i32)?;
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        let mut words = Vec::new();
        for line in result.Lines()? {
            for word in line.Words()? {
                let r = word.BoundingRect()?;
                words.push(Word { text: word.Text()?.to_string(), x: r.X / scale, y: r.Y / scale, w: r.Width / scale, h: r.Height / scale });
            }
        }
        Ok(words)
    };
    recognise().map_err(|e| format!("OCR failed: {}", e.message()))
}

#[test]
fn prints_to_microsoft_print_to_pdf() {
    // Windows ships this printer; it writes the printed pages to a PDF file
    const PRINTER: &str = "Microsoft Print to PDF";
    if !printers().iter().any(|p| p == PRINTER) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("printertui-print-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (img, src, out) = (dir.join("page"), dir.join("in.pdf"), dir.join("out.pdf"));
    image::GrayImage::from_pixel(600, 800, image::Luma([80])).save_with_format(&img, image::ImageFormat::Png).unwrap();
    let img = img.to_string_lossy().into_owned();
    std::fs::write(&src, images_to_pdf(&[img.clone(), img.clone(), img.clone(), img], 100, &[]).unwrap()).unwrap();
    let job = Job {
        printer: PRINTER.into(), file: src.to_string_lossy().into_owned(), color: false, paper: "A4",
        pages: Some("1-4".into()), reverse: true, copies: 1, collate: true, per_sheet: 2,
    };
    print_pdf(&job, Some(&out.to_string_lossy())).unwrap();
    // the spooler writes the file after EndDoc returns
    let out = out.to_string_lossy().into_owned();
    for _ in 0..50 {
        if page_count(&out).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    // 4 pages, 2 per sheet side
    assert_eq!(page_count(&out), Some(2));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn system_bits() {
    let t = timestamp();
    assert_eq!(t.len(), 17, "{t}");
    assert!(config_path().unwrap().ends_with(r"printertui\config"));
    let _ = queue();
    let _ = local_scanners();
}

#[test]
fn renders_a_preview_page() {
    let dir = std::env::temp_dir().join(format!("printertui-render-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
    image::RgbImage::from_pixel(200, 100, image::Rgb([0, 0, 0])).save(p("black.png")).unwrap();
    std::fs::write(p("in.pdf"), images_to_pdf(&[p("black.png")], 100, &[]).unwrap()).unwrap();
    // scaled to half: the page stays white at the corner and black in the middle
    render_page(&scale_pdf(&p("in.pdf"), 50).unwrap(), 1, &p("page.png")).unwrap();
    let img = image::open(p("page.png")).unwrap().to_luma8();
    assert_eq!(img.width(), 1000);
    assert!(img.get_pixel(2, 2)[0] > 200 && img.get_pixel(500, img.height() / 2)[0] < 50);
    std::fs::remove_dir_all(&dir).unwrap();
}
