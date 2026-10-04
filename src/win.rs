//! Windows: the print spooler (winspool), pdfium to draw PDF pages on the printer, WinRT for
//! scanners and OCR, and PowerShell's Add-Printer for network printers.

use crate::*;
use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::Scanners::{ImageScanner, ImageScannerColorMode, ImageScannerFormat, ImageScannerResolution, ImageScannerScanSource};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::StorageFolder;
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::DEVPROPTYPE;
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Printing::*;
use windows::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// Windows' own resolver finds .local names (mDNS), so every name is left as it is.
pub fn resolve_host(host: &str) -> Option<String> {
    Some(host.to_string())
}

/// No paper is suggested from the printer: Windows starts with the first of PAPERS.
pub fn default_paper(_queue: &str) -> Option<&'static str> {
    None
}

/// The user's first display language, like "es-ES".
pub fn system_language() -> String {
    windows::Globalization::ApplicationLanguages::Languages().and_then(|l| l.GetAt(0)).map(|l| l.to_string()).unwrap_or_default()
}

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

/// Installed printers as (name, port, attributes, status).
fn installed() -> Vec<(String, String, u32, u32)> {
    let (buf, n) =
        enum_buffer(|b, need, count| unsafe { EnumPrintersW(PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS, PCWSTR::null(), 2, b, need, count).is_ok() });
    let infos = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const PRINTER_INFO_2W, n) };
    infos.iter().map(|p| (text(p.pPrinterName), text(p.pPortName), p.Attributes, p.Status)).collect()
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
    let mut list: Vec<String> = installed().into_iter().map(|(name, ..)| name).collect();
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
        .map(|q| match all.iter().find(|(name, ..)| name == q).and_then(|(_, port, ..)| printer_uri(q, port)).as_deref().and_then(uri_host) {
            Some((_, host)) => format!("{q} ({host})"),
            None => q.clone(),
        })
        .collect()
}

/// The IPP address of a network printer: from its port ("IP_192.168.1.46"), or for the ones this
/// app added, whose port is just "WSD-<id>", from the note it made then.
fn printer_uri(queue: &str, port: &str) -> Option<String> {
    port_host(port).map(|host| format!("ipp://{host}/ipp/print")).or_else(|| added().into_iter().find(|(name, _)| name == queue).map(|(_, uri)| uri))
}

/// Where the printers this app added are noted, with their address: next to the settings.
fn added_file() -> Option<std::path::PathBuf> {
    Some(config_path()?.with_file_name("printers"))
}

/// The printers this app added, as (name, IPP address): one "name<TAB>address" line each.
fn added() -> Vec<(String, String)> {
    let text = added_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    text.lines().filter_map(|l| l.split_once('\t')).map(|(n, u)| (n.to_string(), u.to_string())).collect()
}

fn note_added(name: &str, uri: &str) {
    let Some(file) = added_file() else { return };
    let mut list: Vec<(String, String)> = added().into_iter().filter(|(n, _)| n != name).collect();
    list.push((name.to_string(), uri.to_string()));
    let text: String = list.iter().map(|(n, u)| format!("{n}\t{u}\n")).collect();
    let _ = file.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(file, text);
}

/// Hosts of the installed network printers, to try as eSCL scanners.
pub fn printer_hosts() -> Vec<String> {
    let mut hosts: Vec<String> = installed().iter().filter_map(|(_, port, ..)| port_host(port)).collect();
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
    let mut path = dir.join("pdfium-7881.dll");
    // byte for byte, so a damaged or half-written copy is never loaded
    if std::fs::read(&path).ok().as_deref() != Some(DLL) {
        let err = |e: std::io::Error| format!("{}: {e}", dir.display());
        std::fs::create_dir_all(&dir).map_err(err)?;
        let tmp = dir.join(format!("pdfium-7881-{}.dll", std::process::id()));
        std::fs::write(&tmp, DLL).map_err(err)?;
        // another PrinterTUI may have the old copy loaded, and Windows will not replace it then:
        // this one loads its own copy
        match std::fs::rename(&tmp, &path) {
            Ok(()) => {}
            Err(_) => path = tmp,
        }
    }
    let bindings = Pdfium::bind_to_library(&path).map_err(|e| format!("pdfium: {e}"))?;
    Ok(PDFIUM.get_or_init(|| Pdfium::new(bindings)))
}

/// Printer settings for the job: paper, color, copies, the sheet turned sideways for 2 and 6
/// pages per sheet, and always one-sided (double-sided is done by hand). Also returns whether
/// the driver makes the copies itself, as asked; else each copy is drawn again.
fn devmode(printer: &str, job: &Job) -> Result<(Vec<u64>, bool), String> {
    let name = wide(printer);
    with_printer(printer, |h| unsafe {
        let size = DocumentPropertiesW(None, h, PCWSTR(name.as_ptr()), None, None, 0);
        if size <= 0 {
            return Err(format!("{printer}: cannot read its settings"));
        }
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let dm = buf.as_mut_ptr() as *mut DEVMODEW;
        if DocumentPropertiesW(None, h, PCWSTR(name.as_ptr()), Some(dm), None, DM_OUT_BUFFER.0) < 0 {
            return Err(format!("{printer}: cannot read its settings"));
        }
        let paper = match job.paper {
            "Letter" => DMPAPER_LETTER,
            "Legal" => DMPAPER_LEGAL,
            "A5" => DMPAPER_A5,
            "A3" => DMPAPER_A3,
            _ => DMPAPER_A4,
        };
        let copies = job.copies.clamp(1, i16::MAX as u32) as i16;
        let (_, _, landscape) = sheet_grid(job.per_sheet);
        (*dm).Anonymous1.Anonymous1.dmPaperSize = paper as i16;
        (*dm).Anonymous1.Anonymous1.dmOrientation = if landscape { DMORIENT_LANDSCAPE } else { DMORIENT_PORTRAIT } as i16;
        (*dm).Anonymous1.Anonymous1.dmCopies = copies;
        (*dm).dmCollate = if job.collate { DMCOLLATE_TRUE } else { DMCOLLATE_FALSE };
        (*dm).dmDuplex = DMDUP_SIMPLEX;
        // color is asked for either way: a driver may print gray unless told
        (*dm).dmColor = if job.color { DMCOLOR_COLOR } else { DMCOLOR_MONOCHROME };
        (*dm).dmFields |= DM_PAPERSIZE | DM_ORIENTATION | DM_COPIES | DM_COLLATE | DM_DUPLEX | DM_COLOR;
        // let the driver check the changes against what the printer can do
        if DocumentPropertiesW(None, h, PCWSTR(name.as_ptr()), Some(dm), Some(dm), DM_IN_BUFFER.0 | DM_OUT_BUFFER.0) < 0 {
            return Err(format!("{printer}: refused the print settings"));
        }
        // a driver that cannot make copies (or collate them) puts its own values back
        let kept = (*dm).Anonymous1.Anonymous1.dmCopies == copies && (copies == 1 || !job.collate || (*dm).dmCollate == DMCOLLATE_TRUE);
        if !kept {
            (*dm).Anonymous1.Anonymous1.dmCopies = 1;
        }
        Ok((buf, kept))
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
    let (dm, driver_copies) = devmode(&job.printer, job)?;
    let copies = if driver_copies { 1 } else { job.copies.max(1) as usize };
    let order: Vec<&[u32]> = if job.collate {
        (0..copies).flat_map(|_| sheets.iter().copied()).collect()
    } else {
        sheets.iter().flat_map(|s| std::iter::repeat_n(*s, copies)).collect()
    };

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
            if res.is_ok() {
                EndDoc(hdc)
            } else {
                AbortDoc(hdc)
            };
        }
        res.map(|_| id as u32)
    };
    let _ = unsafe { DeleteDC(hdc) };
    res
}

/// One sheet side: its pages placed by `place_pages`, each rendered at up to 300 dpi.
fn draw_sheet(hdc: HDC, doc: &PdfDocument, sheet: &[u32], job: &Job) -> Result<(), String> {
    let cap = |i| unsafe { GetDeviceCaps(Some(hdc), i) };
    let dpi = (cap(LOGPIXELSX).max(72) as f32, cap(LOGPIXELSY).max(72) as f32);
    // rendering at the printer's full resolution would need hundreds of MB per page
    let render_dpi = dpi.0.max(dpi.1).min(300.0);
    let pages = sheet.iter().map(|&n| doc.pages().get((n - 1) as PdfPageIndex).map_err(|e| format!("page {n}: {e}"))).collect::<Result<Vec<_>, _>>()?;
    let sizes: Vec<(f32, f32)> = pages.iter().map(|p| (p.width().value, p.height().value)).collect();
    let places = place_pages(&sizes, job.per_sheet, (cap(HORZRES), cap(VERTRES)), dpi);
    if unsafe { StartPage(hdc) } <= 0 {
        return Err("The printer stopped accepting pages".into());
    }
    unsafe { SetStretchBltMode(hdc, HALFTONE) };
    for ((page, &n), &(x, y, dw, dh, turn)) in pages.iter().zip(sheet).zip(&places) {
        // pixels for the page at render_dpi, on the paper's scale
        let factor = dw as f32 / dpi.0 * render_dpi / if turn { page.height().value } else { page.width().value };
        let mut config = PdfRenderConfig::new()
            .scale_page_by_factor(factor)
            .set_format(PdfBitmapFormat::BGRA)
            .use_grayscale_rendering(!job.color)
            .use_print_quality(true)
            .render_form_data(true);
        if turn {
            // a quarter turn to the left: the top of the page along the left edge
            config = config.rotate(PdfPageRenderRotation::Degrees270, true);
        }
        let bitmap = page.render_with_config(&config).map_err(|e| format!("page {n}: {e}"))?;
        let (bw, bh) = (bitmap.width(), bitmap.height());
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

/// Spooler status bits and the IPP printer-state-reasons they stand for.
const PROBLEMS: [(u32, &str); 8] = [
    (PRINTER_STATUS_PAUSED, "stopped"),
    (PRINTER_STATUS_PAPER_JAM, "media-jam"),
    (PRINTER_STATUS_PAPER_OUT, "media-empty"),
    (PRINTER_STATUS_OFFLINE, "offline"),
    (PRINTER_STATUS_NOT_AVAILABLE, "offline"),
    (PRINTER_STATUS_DOOR_OPEN, "door-open"),
    (PRINTER_STATUS_NO_TONER, "marker-supply-empty"),
    (PRINTER_STATUS_OUTPUT_BIN_FULL, "output-area-full"),
];

/// The printer's problems from the spooler, and its ink asked over IPP when it is on the network.
// ponytail: network printers Windows added by itself (a "WSD-..." port) have no address here, so no ink
pub fn printer_state(queue: &str) -> PrinterState {
    let Some((_, port, _, status)) = installed().into_iter().find(|(name, ..)| name == queue) else {
        return PrinterState::default();
    };
    let mut state = printer_uri(queue, &port)
        .and_then(|uri| ipp_attributes(&uri, &STATE_ATTRIBUTES).ok())
        .map_or_else(PrinterState::default, |attrs| PrinterState::from(&attrs));
    for (bit, problem) in PROBLEMS {
        if status & bit != 0 && !state.problems.iter().any(|p| p == problem) {
            state.problems.push(problem.into());
        }
    }
    state
}

/// A photo in a format Windows reads but this app does not (iPhone's HEIC, AVIF, JPEG XR, camera
/// raw) as a PNG, converted by Windows' own decoders; None for other files. HEIC and AVIF need
/// the free HEIF and AV1 extensions from the Microsoft Store, which Windows 11 usually has.
pub fn convert_photo(file: &str) -> Option<Result<String, String>> {
    let ext = std::path::Path::new(file).extension()?.to_string_lossy().to_lowercase();
    let raw = ["dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf"];
    if !matches!(ext.as_str(), "heic" | "heif" | "hif" | "avif" | "jxr" | "wdp" | "hdp") && !raw.contains(&ext.as_str()) {
        return None;
    }
    Some(work_dir(file).and_then(|dir| {
        let out = dir.join("photo.png");
        if !fresh(file, &out) {
            decode_with_windows(file, &out.to_string_lossy()).map_err(|e| {
                format!("{file}: Windows could not read the photo ({e}). HEIC and AVIF photos need the HEIF and AV1 extensions from the Microsoft Store")
            })?;
        }
        Ok(out.to_string_lossy().into_owned())
    }))
}

/// Decodes an image with Windows Imaging Component, turned upright as its EXIF says, into a PNG.
fn decode_with_windows(file: &str, png: &str) -> Result<(), String> {
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapTransform, ColorManagementMode, ExifOrientationMode};
    use windows::Storage::{FileAccessMode, StorageFile, Streams::Buffer};
    let path = std::path::absolute(file).map_err(|e| e.to_string())?;
    let decode = || -> windows::core::Result<(u32, u32, Vec<u8>)> {
        let f = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.join()?;
        let decoder = BitmapDecoder::CreateAsync(&f.OpenAsync(FileAccessMode::Read)?.join()?)?.join()?;
        let bitmap = decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Rgba8,
                BitmapAlphaMode::Ignore,
                &BitmapTransform::new()?,
                ExifOrientationMode::RespectExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )?
            .join()?;
        let (w, h) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
        let buffer = Buffer::Create(w * h * 4)?;
        bitmap.CopyToBuffer(&buffer)?;
        let mut bytes = windows::core::Array::new();
        CryptographicBuffer::CopyToByteArray(&buffer, &mut bytes)?;
        Ok((w, h, bytes.to_vec()))
    };
    let (w, h, rgba) = decode().map_err(|e| e.message())?;
    let img = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad bitmap")?;
    // the alpha channel was ignored, so it may hold anything
    image::DynamicImage::ImageRgba8(img).to_rgb8().save_with_format(png, image::ImageFormat::Png).map_err(|e| format!("{png}: {e}"))
}

/// The Documents folder wherever Windows keeps it (in OneDrive when its backup is on).
pub fn documents_dir() -> Option<std::path::PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().ok().map(std::path::PathBuf::from);
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        path
    }
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
    for (printer, ..) in installed() {
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
pub fn discover() -> Result<Vec<(String, String)>, String> {
    let mdns = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let events = mdns.browse("_ipp._tcp.local.").map_err(|e| e.to_string())?;
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
    Ok(found)
}

/// Adds an IPP printer with Windows' own IPP driver; Windows asks for administrator rights.
/// The name and address can come from anyone on the network, so they only reach PowerShell as
/// quoted literals inside base64-encoded scripts.
pub fn add_printer(name: &str, uri: &str) -> Result<(), String> {
    check_uri(uri)?;
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    // the elevated PowerShell has no window to show an error in, so it leaves it in a file
    let why = std::env::temp_dir().join(format!("printertui-add-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&why);
    let add = format!(
        "$ErrorActionPreference = 'Stop'; try {{ Add-Printer -Name {} -IppURL {} }} catch {{ [IO.File]::WriteAllText({}, $_.Exception.Message); exit 1 }}",
        ps_quote(&name),
        ps_quote(uri),
        ps_quote(&why.to_string_lossy())
    );
    let elevate = format!(
        "try {{ $p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{}'; exit $p.ExitCode }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}",
        ps_encoded(&add)
    );
    let res = run("powershell", &["-NoProfile", "-NonInteractive", "-EncodedCommand", &ps_encoded(&elevate)]);
    let reason = std::fs::read_to_string(&why).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let _ = std::fs::remove_file(&why);
    if let Some(reason) = reason {
        return Err(format!("{name}: {reason}"));
    }
    res?;
    if !printers().contains(&name) {
        return Err(format!("Windows did not add {name}"));
    }
    note_added(&name, uri);
    Ok(())
}

/// The console window, as the file dialog's owner, so the dialog opens in front of the terminal
/// instead of behind it (Windows Terminal hands out a stand-in window for just this).
struct Console;

impl raw_window_handle::HasWindowHandle for Console {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        use raw_window_handle::{HandleError, RawWindowHandle, Win32WindowHandle, WindowHandle};
        let hwnd = unsafe { windows::Win32::System::Console::GetConsoleWindow() };
        let hwnd = std::num::NonZeroIsize::new(hwnd.0 as isize).ok_or(HandleError::Unavailable)?;
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(hwnd))) })
    }
}

impl raw_window_handle::HasDisplayHandle for Console {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::windows())
    }
}

/// The Windows file dialog.
pub fn pick_files() -> Vec<String> {
    rfd::FileDialog::new()
        .set_parent(&Console)
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
pub fn ocr_pdf(pages: &[(String, u32)], out: &str) -> Result<Vec<String>, String> {
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
        "Searchable PDF needs a Windows OCR language: Settings > Time & language > Language, add your language with 'Optical character recognition'".to_string()
    })?;
    let words = pages.iter().map(|(p, _)| ocr_page(&engine, p)).collect::<Result<Vec<_>, _>>()?;
    let path = format!("{out}.pdf");
    std::fs::write(&path, pages_to_pdf(pages, &words)?).map_err(|e| format!("{path}: {e}"))?;
    Ok(vec![path])
}

fn ocr_page(engine: &OcrEngine, page: &str) -> Result<Vec<Word>, String> {
    let img = image::ImageReader::open(page).and_then(|r| r.with_guessed_format()).map_err(|e| e.to_string())?.decode().map_err(|e| e.to_string())?.to_luma8();
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
        printer: PRINTER.into(),
        file: src.to_string_lossy().into_owned(),
        color: false,
        paper: "A4",
        pages: Some("1-4".into()),
        reverse: true,
        copies: 1,
        collate: true,
        per_sheet: 2,
    };
    // the spooler writes the file after EndDoc returns; returns how the first sheet looks
    let print = |job: &Job, out: &std::path::Path| {
        print_pdf(job, Some(&out.to_string_lossy())).unwrap();
        let out = out.to_string_lossy().into_owned();
        for _ in 0..50 {
            if page_count(&out).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let png = format!("{out}.png");
        render_page(&out, 1, &png).unwrap();
        (page_count(&out), image::open(&png).unwrap().to_luma8())
    };
    let dark = |img: &image::GrayImage, x: f32, y: f32| img.get_pixel((x * img.width() as f32) as u32, (y * img.height() as f32) as u32)[0] < 150;

    // 4 pages, 2 per sheet side: the sheet sideways, each page filling its half (not a stamp in the middle)
    let (pages, sheet) = print(&job, &out);
    assert_eq!(pages, Some(2));
    assert!(sheet.width() > sheet.height());
    assert!(dark(&sheet, 0.05, 0.5) && dark(&sheet, 0.25, 0.1) && dark(&sheet, 0.95, 0.5));

    // one 6 x 8 inch page to a sheet prints at its own size: 6 of A4's 8.27 inches across
    let one = Job { pages: Some("1".into()), per_sheet: 1, ..job };
    let (_, sheet) = print(&one, &dir.join("one.pdf"));
    assert!(sheet.height() > sheet.width());
    assert!(dark(&sheet, 0.2, 0.5) && dark(&sheet, 0.8, 0.5) && !dark(&sheet, 0.08, 0.5) && !dark(&sheet, 0.92, 0.5));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn windows_decodes_photos() {
    let dir = std::env::temp_dir().join(format!("printertui-wic-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, out) = (dir.join("in.png"), dir.join("out.png"));
    image::RgbImage::from_fn(30, 20, |x, _| image::Rgb([if x < 15 { 255 } else { 0 }, 0, 200])).save(&src).unwrap();
    decode_with_windows(&src.to_string_lossy(), &out.to_string_lossy()).unwrap();
    let img = image::open(&out).unwrap().to_rgb8();
    assert_eq!(img.dimensions(), (30, 20));
    assert_eq!(img.get_pixel(2, 2).0, [255, 0, 200]);
    assert_eq!(img.get_pixel(28, 2).0, [0, 0, 200]);
    // a HEIC photo goes to Windows; a missing one fails with a reason
    assert!(convert_photo(&dir.join("IMG_0001.HEIC").to_string_lossy()).unwrap().is_err());
    assert!(convert_photo(&src.to_string_lossy()).is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn system_bits() {
    let t = timestamp();
    assert_eq!(t.len(), 17, "{t}");
    assert!(config_path().unwrap().ends_with(r"printertui\config"));
    assert!(documents_dir().unwrap().is_absolute());
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
