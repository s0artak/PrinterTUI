//! Printing, scanning and scan editing. What differs per system (CUPS/SANE, or the Windows
//! print spooler and scanner API) lives in `unix` and `win`, with the same functions.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ColorType, DynamicImage, GrayImage, ImageFormat, Luma};
use pdf_writer::types::TextRenderingMode;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Running child pids, and the eSCL scan job urls this process started; `stop_all` ends them.
static CHILDREN: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static SCAN_JOBS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub const PAPERS: [&str; 5] = ["A4", "Letter", "Legal", "A5", "A3"];
/// Pages per sheet side (`number-up`).
pub const PER_SHEET: [u32; 6] = [1, 2, 4, 6, 9, 16];
/// Content size in percent of the page, see `scale_pdf`.
pub const SCALES: [u32; 9] = [25, 50, 75, 90, 100, 110, 125, 150, 200];

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{cmd}: {e}"))?;
    let pid = child.id();
    CHILDREN.lock().unwrap().push(pid);
    let out = child.wait_with_output();
    CHILDREN.lock().unwrap().retain(|&p| p != pid);
    let out = out.map_err(|e| format!("{cmd}: {e}"))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() {
        return Ok(text(&out.stdout));
    }
    // a tool that fails without a word still gets an error that says something
    let err = [text(&out.stderr), text(&out.stdout)].into_iter().find(|e| !e.is_empty());
    Err(err.unwrap_or_else(|| format!("{cmd}: {}", out.status)))
}

/// A PowerShell string literal. PowerShell ends a '...' string at the typographic quotes ‘ ’ ‚ ‛
/// as well as at ', so all of them are doubled: no name or address can end it early.
#[cfg(any(windows, test))]
fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// A script for `powershell -EncodedCommand`: base64 of its UTF-16 text, which no command line can misread.
#[cfg(any(windows, test))]
fn ps_encoded(script: &str) -> String {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&utf16)
}

/// A printer address to hand to the system: ipp, ipps, http or https, in printable ASCII without
/// quotes, so an address announced on the network cannot carry anything else.
#[cfg(any(windows, test))]
fn check_uri(uri: &str) -> Result<(), String> {
    let scheme = matches!(uri_host(uri), Some(("ipp" | "ipps" | "http" | "https", _)));
    if scheme && uri.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '\'' | '"' | '`')) {
        Ok(())
    } else {
        Err(format!("Not a printer address: {uri}"))
    }
}

/// Page count of a PDF, None for anything that is not one.
pub fn page_count(file: &str) -> Option<u32> {
    let n = lopdf::Document::load(file).ok()?.get_pages().len() as u32;
    (n > 0).then_some(n)
}

/// A temp folder of its own for each source file, so same-named files never overwrite each other.
pub fn work_dir(file: &str) -> Result<std::path::PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    file.hash(&mut h);
    let dir = std::env::temp_dir().join("printertui").join(format!("{:016x}", Hasher::finish(&h)));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// This session's folder for scanned pages, edits and copies; quitting removes it.
pub fn scan_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("printertui-scan").join(std::process::id().to_string())
}

/// Deletes a scanned page's files: the scan, its edited copies and their previews.
pub fn forget_scan(orig: &str) {
    let path = std::path::Path::new(orig);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().into_owned())) else { return };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = entry.file_name().to_string_lossy().into_owned();
        // page-17 and its page-17-r90-f0.png, page-17.preview.png; not page-170
        if n == name || n.strip_prefix(&name).is_some_and(|rest| rest.starts_with(['-', '.'])) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Removes what earlier sessions left in the temp folder a day after it was last written: scans
/// (often private papers) and documents converted for printing. Sounds and LibreOffice's
/// profile stay, as they are only slow to make again.
pub fn clean_temp() {
    let day = std::time::Duration::from_secs(24 * 3600);
    let age = |p: &std::path::Path| p.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
    let tmp = std::env::temp_dir();
    let mine = scan_dir();
    for entry in [tmp.join("printertui-scan"), tmp.join("printertui")].iter().filter_map(|d| std::fs::read_dir(d).ok()).flatten().flatten() {
        let (path, name) = (entry.path(), entry.file_name().to_string_lossy().into_owned());
        // session folders (a pid) and work folders (16 hex digits), and pages older versions left loose
        if !(name.chars().all(|c| c.is_ascii_hexdigit()) || name.starts_with("page-") || name == "copy.pdf") || path == mine {
            continue;
        }
        // a folder counts as written when its newest file was
        let files: Vec<_> = std::fs::read_dir(&path).into_iter().flatten().flatten().filter_map(|e| age(&e.path())).collect();
        let newest = if files.is_empty() { age(&path) } else { files.into_iter().min() };
        if newest.is_some_and(|a| a > day) {
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// True when `out` was written after `src` last changed, so it can be reused.
fn fresh(src: &str, out: &std::path::Path) -> bool {
    let modified = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified());
    matches!((modified(src.as_ref()), modified(out)), (Ok(s), Ok(o)) if o >= s)
}

/// Writes aside and renames, so a print reading the previous version never sees half a file.
fn write_atomic(out: &std::path::Path, data: &[u8]) -> Result<(), String> {
    let tmp = out.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, data).and_then(|_| std::fs::rename(&tmp, out)).map_err(|e| format!("{}: {e}", out.display()))
}

/// Paper size in inches, portrait (width, height).
pub fn paper_inches(paper: &str) -> (f32, f32) {
    match paper {
        "Letter" => (8.5, 11.0),
        "Legal" => (8.5, 14.0),
        "A5" => (5.83, 8.27),
        "A3" => (11.69, 16.54),
        _ => (8.27, 11.69),
    }
}

/// Converts a document, text or image to PDF with LibreOffice, returns the PDF path.
/// The PDF is reused while it is newer than the file, so the preview does not convert it again.
pub fn to_pdf(file: &str) -> Result<String, String> {
    let dir = work_dir(file)?;
    let d = dir.to_str().ok_or("Bad temp dir")?;
    let stem = std::path::Path::new(file).file_stem().ok_or("Bad file name")?;
    let pdf = dir.join(stem).with_extension("pdf").to_string_lossy().into_owned();
    if fresh(file, pdf.as_ref()) && page_count(&pdf).is_some() {
        return Ok(pdf);
    }
    let _ = std::fs::remove_file(&pdf);
    // own profile, so a running LibreOffice window does not swallow the conversion
    // (a file URL: file:///tmp/... or file:///C:/Users/...)
    let base = std::env::temp_dir().join("printertui");
    let profile = format!("-env:UserInstallation=file:///{}/profile", base.to_string_lossy().replace('\\', "/").trim_start_matches('/'));
    run(&soffice(), &[&profile, "--headless", "--convert-to", "pdf", "--outdir", d, file])
        .map_err(|e| format!("Could not convert {file} to PDF (is libreoffice installed?): {e}"))?;
    page_count(&pdf).map(|_| pdf).ok_or(format!("LibreOffice could not convert {file} to PDF"))
}

/// A copy of the PDF with each page's content scaled by `percent` around the page centre; the
/// paper size stays, so above 100 the edges are cut off. Returns the copy's path.
// ponytail: links and form fields (annotations) keep their size and place; scale them too if a form ever prints wrong
pub fn scale_pdf(file: &str, percent: u32) -> Result<String, String> {
    use lopdf::{Dictionary, Object, Stream};
    let err = |e: lopdf::Error| format!("{file}: {e}");
    let mut doc = lopdf::Document::load(file).map_err(err)?;
    let s = percent as f64 / 100.0;
    let pages: Vec<_> = doc.page_iter().collect();
    for id in pages {
        let [x0, y0, x1, y1] = page_box(&doc, id).unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let (dx, dy) = ((x0 + x1) / 2.0 * (1.0 - s), (y0 + y1) / 2.0 * (1.0 - s));
        let mut contents: Vec<Object> = doc.get_page_contents(id).into_iter().map(Object::Reference).collect();
        let pre = doc.add_object(Stream::new(Dictionary::new(), format!("q {s:.4} 0 0 {s:.4} {dx:.4} {dy:.4} cm\n").into_bytes()));
        let post = doc.add_object(Stream::new(Dictionary::new(), b"\nQ\n".to_vec()));
        contents.insert(0, pre.into());
        contents.push(post.into());
        doc.get_dictionary_mut(id).map_err(err)?.set("Contents", contents);
    }
    let out = work_dir(file)?.join(format!("scaled-{percent}.pdf"));
    let mut data = Vec::new();
    doc.save_to(&mut data).map_err(|e| format!("{file}: {e}"))?;
    write_atomic(&out, &data)?;
    Ok(out.to_string_lossy().into_owned())
}

/// The visible area of a page (CropBox, else MediaBox), which pages may inherit from their parents.
fn page_box(doc: &lopdf::Document, page: lopdf::ObjectId) -> Option<[f64; 4]> {
    for key in [&b"CropBox"[..], b"MediaBox"] {
        let mut dict = doc.get_dictionary(page).ok()?;
        loop {
            if let Ok(arr) = dict.get_deref(key, doc).and_then(lopdf::Object::as_array) {
                let v: Vec<f64> = arr.iter().filter_map(|o| o.as_float().ok()).map(f64::from).collect();
                if let [a, b, c, d] = v[..] {
                    return Some([a.min(c), b.min(d), a.max(c), b.max(d)]);
                }
            }
            match dict.get(b"Parent").and_then(lopdf::Object::as_reference).and_then(|p| doc.get_dictionary(p)) {
                Ok(parent) => dict = parent,
                Err(_) => break,
            }
        }
    }
    None
}

/// A photo as a one-page PDF that fills the paper, turned whichever way it prints larger: the
/// print system shrinks big pages to the paper (turning sideways ones), but prints small ones as they are.
fn photo_pdf(file: &str, paper: &str) -> Result<String, String> {
    let out = work_dir(file)?.join(format!("photo-{paper}.pdf"));
    if !fresh(file, &out) {
        let (w, h) = { let img = open(file)?; (img.width() as f32, img.height() as f32) };
        let (pw, ph) = paper_inches(paper);
        let dpi = (w / pw).max(h / ph).min((w / ph).max(h / pw)).ceil().max(1.0);
        write_atomic(&out, &images_to_pdf(&[file.to_string()], dpi as u32, &[])?)?;
    }
    Ok(out.to_string_lossy().into_owned())
}

/// The PDF that gets printed for a file: a photo, a text file or a document converted if it is
/// not one (only documents need LibreOffice), then scaled.
pub fn printable(file: &str, percent: u32, paper: &str) -> Result<String, String> {
    let pdf = if page_count(file).is_some() {
        file.to_string()
    } else if image_size(file).is_ok() {
        // a photo this app decodes itself (JPEG, PNG, BMP, GIF, TIFF, WebP)
        photo_pdf(file, paper)?
    } else if let Some(photo) = convert_photo(file) {
        // iPhone photos (HEIC) and other image formats the system can read
        photo_pdf(&photo?, paper)?
    } else if let Some(text) = plain_text(file) {
        // letters the built-in font has not got print through LibreOffice when it is there
        if text.chars().all(|c| (c as u32) < 256 || c == '\n') { text_pdf(file, &text, paper)? } else { to_pdf(file).or_else(|_| text_pdf(file, &text, paper))? }
    } else {
        to_pdf(file)?
    };
    if percent == 100 { Ok(pdf) } else { scale_pdf(&pdf, percent) }
}

/// The file's text if it is plain text (not markup, which LibreOffice lays out better).
fn plain_text(file: &str) -> Option<String> {
    let ext = std::path::Path::new(file).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "html" | "htm" | "xhtml" | "rtf" | "svg" | "xml") || std::fs::metadata(file).ok()?.len() > 20 << 20 {
        return None;
    }
    String::from_utf8(std::fs::read(file).ok()?).ok().filter(|t| !t.contains('\0'))
}

/// Plain text on pages of the paper in Courier 10 pt, long lines wrapped.
fn text_pdf(file: &str, text: &str, paper: &str) -> Result<String, String> {
    const MARGIN: f32 = 56.0;
    const SIZE: f32 = 10.0;
    const LEADING: f32 = 12.0;
    let (pw, ph) = paper_inches(paper);
    let (pw, ph) = (pw * 72.0, ph * 72.0);
    // Courier is monospaced: every letter is 0.6 of the font size wide
    let cols = ((pw - 2.0 * MARGIN) / (0.6 * SIZE)) as usize;
    let rows = ((ph - 2.0 * MARGIN) / LEADING) as usize;
    let mut lines = Vec::new();
    for line in text.replace('\t', "    ").replace('\r', "").lines() {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            lines.push(String::new());
        }
        lines.extend(chars.chunks(cols.max(1)).map(|c| c.iter().collect::<String>()));
    }
    let pages: Vec<&[String]> = if lines.is_empty() { vec![&[][..]] } else { lines.chunks(rows.max(1)).collect() };
    let mut pdf = Pdf::new();
    let (catalog, tree, font) = (Ref::new(1), Ref::new(2), Ref::new(3));
    let mut kids = Vec::new();
    for (i, page_lines) in pages.iter().enumerate() {
        let (page_id, content_id) = (Ref::new(2 * i as i32 + 4), Ref::new(2 * i as i32 + 5));
        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, pw, ph)).parent(tree).contents(content_id);
        page.resources().fonts().pair(Name(b"F0"), font);
        page.finish();
        let mut content = Content::new();
        content.begin_text().set_font(Name(b"F0"), SIZE).set_leading(LEADING).next_line(MARGIN, ph - MARGIN - SIZE);
        for line in page_lines.iter() {
            content.show(Str(&win_ansi(line))).next_line_using_leading();
        }
        content.end_text();
        pdf.stream(content_id, &content.finish());
        kids.push(page_id);
    }
    pdf.type1_font(font).base_font(Name(b"Courier")).encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).count(kids.len() as i32).kids(kids);
    let out = work_dir(file)?.join(format!("text-{paper}.pdf"));
    write_atomic(&out, &pdf.finish())?;
    Ok(out.to_string_lossy().into_owned())
}

/// Width and height of an image, whatever its file name.
pub fn image_size(path: &str) -> Result<(u32, u32), String> {
    image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| format!("{path}: {e}"))?.into_dimensions().map_err(|e| format!("{path}: {e}"))
}

/// Pages per sheet side as (columns, rows, sideways): 2 and 6 go on a sheet turned sideways,
/// side by side, as CUPS lays them out (and Windows prints them, see win.rs).
pub fn sheet_grid(per_sheet: u32) -> (u32, u32, bool) {
    match per_sheet {
        2 => (2, 1, true),
        4 => (2, 2, false),
        6 => (3, 2, true),
        9 => (3, 3, false),
        16 => (4, 4, false),
        _ => (1, 1, false),
    }
}

/// Where each page of a sheet side goes, in the printer's dots: (x, y, width, height, turned).
/// `area` is the printable area in dots and `dpi` the dots per inch across and down, which need
/// not be the same (600 x 1200 dpi). A page turns a quarter when it then fits its cell better (a
/// landscape page on upright paper). One to a sheet it prints at its own size when that fits
/// and is shrunk to fit when not, as CUPS does; several to a sheet they fill their cells.
#[cfg(any(windows, test))]
fn place_pages(pages: &[(f32, f32)], per_sheet: u32, area: (i32, i32), dpi: (f32, f32)) -> Vec<(i32, i32, i32, i32, bool)> {
    let (cols, rows, _) = sheet_grid(per_sheet);
    let (cell_w, cell_h) = (area.0 / cols as i32, area.1 / rows as i32);
    // the cell in inches
    let (cw, ch) = (cell_w as f32 / dpi.0, cell_h as f32 / dpi.1);
    pages
        .iter()
        .enumerate()
        .map(|(k, &(pw, ph))| {
            // the page in inches, as it lands on the paper
            let turn = (pw > ph) != (cw > ch);
            let (w, h) = if turn { (ph / 72.0, pw / 72.0) } else { (pw / 72.0, ph / 72.0) };
            let fit = (cw / w).min(ch / h);
            let scale = if per_sheet == 1 { fit.min(1.0) } else { fit };
            let (dw, dh) = ((w * scale * dpi.0) as i32, (h * scale * dpi.1) as i32);
            let (col, row) = (k as i32 % cols as i32, k as i32 / cols as i32);
            (col * cell_w + (cell_w - dw) / 2, row * cell_h + (cell_h - dh) / 2, dw, dh, turn)
        })
        .collect()
}

/// One sheet side for the preview: the page images in their grid, left to right and top to
/// bottom, each turned a quarter to the left when it then fits its cell better.
pub fn sheet_png(pages: &[String], paper: &str, per_sheet: u32, out: &str) -> Result<(), String> {
    let (cols, rows, sideways) = sheet_grid(per_sheet);
    let (pw, ph) = paper_inches(paper);
    let (pw, ph) = if sideways { (ph, pw) } else { (pw, ph) };
    let px = 1400.0 / pw.max(ph);
    let (w, h) = ((pw * px) as u32, (ph * px) as u32);
    let (cw, ch) = (w / cols, h / rows);
    let mut sheet = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
    for (k, page) in pages.iter().enumerate() {
        let img = open(page)?;
        let img = if (img.width() > img.height()) != (cw > ch) { img.rotate270() } else { img };
        let img = img.resize(cw - 8, ch - 8, FilterType::Triangle).to_rgb8();
        let (x, y) = ((k as u32 % cols) * cw + (cw - img.width()) / 2, (k as u32 / cols) * ch + (ch - img.height()) / 2);
        image::imageops::overlay(&mut sheet, &img, x.into(), y.into());
        // an outline, or white pages vanish into the white sheet
        let edge = image::Rgb([170, 170, 170]);
        for i in 0..img.width() {
            sheet.put_pixel(x + i, y, edge);
            sheet.put_pixel(x + i, y + img.height() - 1, edge);
        }
        for j in 0..img.height() {
            sheet.put_pixel(x, y + j, edge);
            sheet.put_pixel(x + img.width() - 1, y + j, edge);
        }
    }
    save_png(&DynamicImage::ImageRgb8(sheet), out)
}

/// Paths in the File field, separated by ';'. A path may be in quotes, as Windows copies it
/// ("Copy as path") and terminals paste dragged files: "C:\My files\a.pdf" "C:\b.pdf".
pub fn split_files(field: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut path = String::new();
    let mut quote = None;
    let mut done = |path: &mut String| {
        if !path.trim().is_empty() {
            files.push(path.trim().to_string());
        }
        path.clear();
    };
    for c in field.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                done(&mut path);
            }
            Some(_) => path.push(c),
            // a quote opens a path only at its start, so "Ana's notes.txt" stays as it is
            None if matches!(c, '"' | '\'') && path.trim().is_empty() => {
                path.clear();
                quote = Some(c);
            }
            None if c == ';' => done(&mut path),
            None => path.push(c),
        }
    }
    done(&mut path);
    files
}

/// Expands "1-3,7,9-" into a sorted page list. Empty or "all" means every page.
pub fn parse_ranges(spec: &str, total: u32) -> Result<Vec<u32>, String> {
    let spec = spec.trim();
    if spec.is_empty() || spec == "all" {
        return Ok((1..=total).collect());
    }
    let bad = || format!("Invalid page range: {spec}");
    let num = |s: &str, dflt: u32| -> Result<u32, String> {
        if s.is_empty() { Ok(dflt) } else { s.parse().map_err(|_| bad()) }
    };
    let mut pages = Vec::new();
    for part in spec.split(',').map(str::trim) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (num(a.trim(), 1)?, num(b.trim(), total)?),
            None => { let n = num(part, 0)?; (n, n) }
        };
        if a == 0 || a > b || b > total {
            return Err(format!("Pages must be within 1-{total}: {part}"));
        }
        pages.extend(a..=b);
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

/// Splits sheet sides into (front, back) for manual duplex: 1st, 3rd, 5th... on the front.
pub fn split_duplex<T: Clone>(sides: &[T]) -> (Vec<T>, Vec<T>) {
    let front = sides.iter().step_by(2).cloned().collect();
    let back = sides.iter().skip(1).step_by(2).cloned().collect();
    (front, back)
}

/// Sorted pages back to a range string: [1, 2, 3, 7] -> "1-3,7".
pub fn join(pages: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = i;
        while i + 1 < pages.len() && pages[i + 1] == pages[i] + 1 {
            i += 1;
        }
        out.push(if i > start { format!("{}-{}", pages[start], pages[i]) } else { pages[i].to_string() });
        i += 1;
    }
    out.join(",")
}

#[derive(Clone)]
pub struct Job {
    pub printer: String,
    pub file: String,
    pub color: bool,
    pub paper: &'static str,
    pub pages: Option<String>,
    pub reverse: bool,
    pub copies: u32,
    pub collate: bool,
    pub per_sheet: u32,
}

/// Settings file: $XDG_CONFIG_HOME/printertui/config, else ~/.config/printertui/config;
/// %APPDATA%\printertui\config on Windows.
pub fn config_path() -> Option<std::path::PathBuf> {
    let var = |k| std::env::var_os(k).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let base = if cfg!(windows) { var("APPDATA")? } else { var("XDG_CONFIG_HOME").or_else(|| Some(std::env::home_dir()?.join(".config")))? };
    Some(base.join("printertui").join("config"))
}

/// `key=value` lines as trimmed pairs; other lines are skipped.
pub fn parse_config(text: &str) -> Vec<(&str, &str)> {
    // Notepad and Windows PowerShell start UTF-8 files with a byte order mark
    let text = text.trim_start_matches('\u{feff}');
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect()
}

/// On quit: kills running tools (their children first, e.g. LibreOffice's soffice.bin, so none
/// are left orphaned) and cancels the scan running on the scanner. Print jobs already sent keep
/// printing, as with any other app; the queue popup and the system can still cancel them.
pub fn stop_all() {
    let pids = CHILDREN.lock().unwrap().clone();
    if !pids.is_empty() {
        kill_tree(&pids);
    }
    for job in std::mem::take(&mut *SCAN_JOBS.lock().unwrap()) {
        let _ = minreq::delete(job).with_timeout(2).send();
    }
    let _ = std::fs::remove_dir_all(scan_dir());
}

/// "request id is P-12 (1 file(s))" -> "P-12".
pub fn job_id(lp_out: &str) -> Option<&str> {
    lp_out.strip_prefix("request id is ")?.split_whitespace().next()
}

/// Printer name for a network printer at `host`: printer_192_168_1_46.
pub fn queue_name(host: &str) -> String {
    format!("printer_{}", host.replace(|c: char| !c.is_ascii_alphanumeric(), "_"))
}

/// ("ipp", "192.168.1.46") from "ipp://192.168.1.46/ipp/print".
pub fn uri_host(uri: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = uri.split_once("://")?;
    Some((scheme, rest.split(['/', ':', '?']).next().filter(|h| !h.is_empty())?))
}

/// A value of an IPP attribute: integers and enums, or text (keywords, names, uris).
#[derive(Debug, Clone, PartialEq)]
pub enum IppValue {
    Int(i32),
    Text(String),
}

pub type IppAttributes = std::collections::HashMap<String, Vec<IppValue>>;

/// What the printer state shows: ink, and the problems that need a person.
pub const STATE_ATTRIBUTES: [&str; 5] = ["printer-state", "printer-state-reasons", "marker-colors", "marker-levels", "marker-low-levels"];

/// Asks a printer for attributes over IPP (Get-Printer-Attributes). `url` is its ipp://,
/// ipps:// or http:// address; the request goes over plain HTTP, port 631 unless one is given.
pub fn ipp_attributes(url: &str, names: &[&str]) -> Result<IppAttributes, String> {
    let (_, rest) = url.split_once("://").ok_or(format!("Bad printer address: {url}"))?;
    let (authority, path) = rest.find('/').map_or((rest, "/ipp/print"), |i| (&rest[..i], &rest[i..]));
    let authority = if authority.contains(':') { authority.to_string() } else { format!("{authority}:631") };
    let mut body = vec![1, 1, 0, 0x0B, 0, 0, 0, 1, 0x01];
    let mut attr = |tag: u8, name: &str, value: &[u8]| {
        body.push(tag);
        body.extend((name.len() as u16).to_be_bytes());
        body.extend(name.as_bytes());
        body.extend((value.len() as u16).to_be_bytes());
        body.extend(value);
    };
    attr(0x47, "attributes-charset", b"utf-8");
    attr(0x48, "attributes-natural-language", b"en");
    attr(0x45, "printer-uri", format!("ipp://{authority}{path}").as_bytes());
    for (i, name) in names.iter().enumerate() {
        // more values of the same attribute have no name
        attr(0x44, if i == 0 { "requested-attributes" } else { "" }, name.as_bytes());
    }
    body.push(0x03);
    let res = minreq::post(format!("http://{authority}{path}"))
        .with_header("Content-Type", "application/ipp")
        .with_body(body)
        .with_timeout(3)
        .send()
        .map_err(|e| e.to_string())?;
    parse_ipp(res.as_bytes()).ok_or(format!("{url}: not an IPP answer"))
}

/// An IPP response's attributes; None when it is malformed or reports an error.
fn parse_ipp(data: &[u8]) -> Option<IppAttributes> {
    let take = |i: &mut usize, n: usize| {
        let part = data.get(*i..*i + n)?;
        *i += n;
        Some(part)
    };
    let short = |i: &mut usize| take(i, 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize);
    if u16::from_be_bytes([*data.get(2)?, *data.get(3)?]) >= 0x0400 {
        return None;
    }
    let (mut i, mut attrs, mut last) = (8, IppAttributes::new(), String::new());
    while let Some(&tag) = data.get(i) {
        i += 1;
        match tag {
            0x03 => break,
            // the start of an attribute group
            0x00..=0x0F => continue,
            _ => {}
        }
        let n = short(&mut i)?;
        let name = String::from_utf8_lossy(take(&mut i, n)?).into_owned();
        let n = short(&mut i)?;
        let value = take(&mut i, n)?;
        if !name.is_empty() {
            last = name;
        }
        let value = match (tag, value) {
            (0x21 | 0x23, &[a, b, c, d]) => IppValue::Int(i32::from_be_bytes([a, b, c, d])),
            _ => IppValue::Text(String::from_utf8_lossy(value).into_owned()),
        };
        attrs.entry(last.clone()).or_default().push(value);
    }
    Some(attrs)
}

/// What a printer reports about itself.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PrinterState {
    /// Each ink or toner as (RGB color, percent or -1 when unknown, running low).
    pub ink: Vec<(u32, i32, bool)>,
    /// What needs a person, as IPP printer-state-reasons without their -error / -warning
    /// ending: "media-empty", "media-jam", "door-open", "offline", "stopped"...
    pub problems: Vec<String>,
}

impl PrinterState {
    /// From IPP attributes, or from CUPS' copy of them (see `STATE_ATTRIBUTES`).
    pub fn from(attrs: &IppAttributes) -> PrinterState {
        let ints = |key: &str| -> Vec<i32> {
            attrs.get(key).into_iter().flatten().map(|v| match v {
                IppValue::Int(n) => *n,
                IppValue::Text(t) => t.trim().parse().unwrap_or(-1),
            }).collect()
        };
        let texts = |key: &str| -> Vec<String> {
            attrs.get(key).into_iter().flatten().filter_map(|v| match v {
                IppValue::Text(t) => Some(t.clone()),
                IppValue::Int(_) => None,
            }).collect()
        };
        let (levels, lows) = (ints("marker-levels"), ints("marker-low-levels"));
        let ink = texts("marker-colors")
            .iter()
            .zip(&levels)
            .enumerate()
            .map(|(i, (color, &level))| {
                // a marker with several colors ("#00FFFF#FF00FF") shows the first
                let rgb = color.get(1..7).and_then(|c| u32::from_str_radix(c, 16).ok()).unwrap_or(0x808080);
                let level = if level < 0 { -1 } else { level };
                (rgb, level, level >= 0 && level <= lows.get(i).copied().unwrap_or(10).max(10))
            })
            .collect();
        let mut problems: Vec<String> = texts("printer-state-reasons")
            .iter()
            // -report ones are just news
            .filter(|r| !r.ends_with("-report"))
            .map(|r| r.trim_end_matches("-error").trim_end_matches("-warning").to_string())
            // low ink shows on the tanks; CUPS' own notes are not the printer's
            .filter(|r| r != "none" && !r.starts_with("cups-") && !r.ends_with("-low"))
            .collect();
        // 5: stopped (a paused queue, or a printer that gave up)
        if ints("printer-state") == [5] {
            problems.push("stopped".into());
        }
        problems.dedup();
        PrinterState { ink, problems }
    }
}

pub const SCAN_MODES: [&str; 2] = ["Color", "Gray"];
pub const SCAN_DPI: [u32; 3] = [150, 300, 600];
/// Save formats: one PDF, one PDF with a text layer (OCR), or one PNG per page.
pub const SCAN_FORMATS: [&str; 3] = ["PDF", "OCR", "PNG"];

/// Scanners as (device, description). Network printers already installed are tried first
/// as eSCL scanners by address, which is instant and reliable; the system's own discovery
/// (SANE or Windows, slow, finds USB scanners too) runs when `full` is set or nothing was found that way.
pub fn scanners(full: bool) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = printer_hosts()
        .iter()
        .filter_map(|host| {
            let caps = minreq::get(format!("http://{host}/eSCL/ScannerCapabilities")).with_timeout(2).send().ok()?;
            let caps = caps.as_str().ok().filter(|_| caps.status_code == 200)?;
            let model = caps.split("<pwg:MakeAndModel>").nth(1)?.split('<').next()?.to_string();
            Some((format!("escl:http://{host}/eSCL"), model))
        })
        .collect();
    if full || found.is_empty() {
        for (d, v) in local_scanners() {
            // the same device found by address already is the reliable entry
            if !found.iter().any(|(_, model)| v.contains(model.as_str())) {
                found.push((d, v));
            }
        }
    }
    found
}

/// Scans one page from the flatbed into an image file (PNG or JPEG). `escl:<url>` devices are
/// driven directly over HTTP, which also works on macOS where there is no SANE AirScan backend;
/// any other device goes through the system (SANE's `scanimage`, or Windows' scanner API).
pub fn scan(device: &str, mode: &str, dpi: u32, out: &str) -> Result<(), String> {
    match device.strip_prefix("escl:") {
        Some(url) => escl_scan(url, mode, dpi, out),
        None => scan_local(device, mode, dpi, out),
    }
}

/// eSCL (AirScan): POST the settings to ScanJobs, then download the page from the job's NextDocument.
fn escl_scan(url: &str, mode: &str, dpi: u32, out: &str) -> Result<(), String> {
    let color = if mode == "Gray" { "Grayscale8" } else { "RGB24" };
    // A4 in 1/300 inch
    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScanSettings xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
<pwg:Version>2.0</pwg:Version>
<scan:Intent>Document</scan:Intent>
<pwg:ScanRegions><pwg:ScanRegion><pwg:Height>3508</pwg:Height><pwg:Width>2480</pwg:Width><pwg:XOffset>0</pwg:XOffset><pwg:YOffset>0</pwg:YOffset><pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits></pwg:ScanRegion></pwg:ScanRegions>
<pwg:InputSource>Platen</pwg:InputSource>
<scan:ColorMode>{color}</scan:ColorMode>
<scan:XResolution>{dpi}</scan:XResolution>
<scan:YResolution>{dpi}</scan:YResolution>
<pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat>
</scan:ScanSettings>"#
    );
    // right after a page the scanner answers 503 for a few seconds while the head returns
    let res = retry_busy(15, || minreq::post(format!("{url}/ScanJobs")).with_header("Content-Type", "text/xml").with_body(settings.as_str()).with_timeout(30).send())
        .map_err(|e| format!("The scanner refused the scan job (busy?) {e}"))?;
    let job = res.header("location").map(str::trim).ok_or("The scanner did not return a scan job")?;
    // Location may be relative to the scanner ("/eSCL/ScanJobs/..."), keep scheme://host from the url
    let job = if job.starts_with('/') { format!("{}{job}", url.splitn(4, '/').take(3).collect::<Vec<_>>().join("/")) } else { job.to_string() };
    SCAN_JOBS.lock().unwrap().push(job.clone());
    // the scanner answers 503 until the page is ready
    let res = retry_busy(30, || minreq::get(format!("{job}/NextDocument")).with_timeout(600).send())
        .and_then(|page| std::fs::write(out, page.as_bytes()).map_err(|e| e.to_string()));
    SCAN_JOBS.lock().unwrap().retain(|j| *j != job);
    res.map_err(|e| format!("Could not download the scanned page: {e}"))
}

/// Sends a request until it gets a 2xx answer, waiting 2 s after each 503 (busy), `tries` times at most.
fn retry_busy(tries: u32, send: impl Fn() -> Result<minreq::Response, minreq::Error>) -> Result<minreq::Response, String> {
    for _ in 1..tries {
        match send() {
            Ok(r) if r.status_code == 503 => std::thread::sleep(std::time::Duration::from_secs(2)),
            res => return answer(res),
        }
    }
    answer(send())
}

fn answer(res: Result<minreq::Response, minreq::Error>) -> Result<minreq::Response, String> {
    match res {
        Ok(r) if (200..300).contains(&r.status_code) => Ok(r),
        Ok(r) => Err(format!("HTTP {} {}", r.status_code, r.reason_phrase)),
        Err(e) => Err(e.to_string()),
    }
}

/// Decodes a PNG or JPEG whatever its file name (scans have no extension), turned upright as its
/// EXIF orientation says (phone photos).
fn open(path: &str) -> Result<DynamicImage, String> {
    let reader = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| format!("{path}: {e}"))?;
    Ok(decode(reader, path)?.0)
}

/// The image and whether it had to be turned upright.
fn decode<R: std::io::BufRead + std::io::Seek>(reader: image::ImageReader<R>, path: &str) -> Result<(DynamicImage, bool), String> {
    use image::ImageDecoder;
    let err = |e: image::ImageError| format!("{path}: {e}");
    let mut dec = reader.into_decoder().map_err(err)?;
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(dec).map_err(err)?;
    img.apply_orientation(orientation);
    Ok((img, orientation != image::metadata::Orientation::NoTransforms))
}

fn save_png(img: &DynamicImage, path: &str) -> Result<(), String> {
    img.save_with_format(path, ImageFormat::Png).map_err(|e| format!("{path}: {e}"))
}

/// A grayscale thumbnail: width, height and pixels.
pub type Thumb = (usize, usize, Vec<u8>);

/// Grayscale thumbnail of an image, plus `<image>.preview.png` in full quality for terminals
/// with the kitty graphics protocol.
pub fn thumbnail(image: &str) -> Result<Thumb, String> {
    let img = open(image)?;
    save_png(&img.resize(1200, u32::MAX, FilterType::Triangle), &format!("{image}.preview.png"))?;
    // thicken text before shrinking, or it fades to near-white at terminal resolution
    let gray = erode(&erode(&img.to_luma8()));
    let h = (gray.height() * 400 / gray.width().max(1)).max(1);
    let small = image::imageops::resize(&gray, 400, h, FilterType::Triangle);
    Ok((400, h as usize, small.into_raw()))
}

/// 3x3 minimum filter: dark strokes grow by one pixel.
fn erode(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    GrayImage::from_fn(w, h, |x, y| {
        let mut min = 255;
        for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
            for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                min = min.min(img.get_pixel(xx, yy)[0]);
            }
        }
        Luma([min])
    })
}

/// Black and white: stretches the gray levels (2% darkest to black, 1% lightest to white) so the
/// threshold works on pale or uneven scans, then cuts at 60%.
fn black_and_white(mut g: GrayImage) -> GrayImage {
    let mut hist = [0u64; 256];
    for p in g.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total = (g.width() as u64 * g.height() as u64) as f64;
    let level = |share: f64| {
        let mut seen = 0;
        (0..256).find(|&v| {
            seen += hist[v];
            seen as f64 >= share * total
        }).unwrap_or(255) as f64
    };
    let (lo, hi) = (level(0.02), level(0.99));
    let cut = lo + 0.6 * (hi - lo).max(1.0);
    for p in g.pixels_mut() {
        p[0] = if p[0] as f64 > cut { 255 } else { 0 };
    }
    g
}

/// The page as JPEG for a PDF: (width, height, grayscale, bytes). JPEG scans go in as they are.
fn jpeg(path: &str) -> Result<(u32, u32, bool, Vec<u8>), String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let reader = image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().map_err(|e| format!("{path}: {e}"))?;
    let (img, turned) = decode(reader, path)?;
    let (w, h) = (img.width(), img.height());
    let gray = matches!(img.color(), ColorType::L8 | ColorType::La8 | ColorType::L16 | ColorType::La16);
    // a JPEG goes in as it is, unless it had to be turned (PDF viewers ignore EXIF)
    if !turned && data.starts_with(&[0xFF, 0xD8]) && matches!(img.color(), ColorType::L8 | ColorType::Rgb8) {
        return Ok((w, h, gray, data));
    }
    let img = if gray { DynamicImage::ImageLuma8(img.to_luma8()) } else { DynamicImage::ImageRgb8(img.to_rgb8()) };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img).map_err(|e| e.to_string())?;
    Ok((w, h, gray, out))
}

/// A recognised word and its box in image pixels, for the invisible text layer of a searchable PDF.
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Text in the PDF's WinAnsi encoding (Latin-1 letters and accents); other characters become '?'.
fn win_ansi(text: &str) -> Vec<u8> {
    text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

/// One PDF page per image; `dpi` sets the page size (pixels / dpi). `words[i]`, when given, is laid
/// invisibly over page i so the PDF can be searched and copied from.
pub fn images_to_pdf(pages: &[String], dpi: u32, words: &[Vec<Word>]) -> Result<Vec<u8>, String> {
    let mut pdf = Pdf::new();
    let (catalog, tree, font) = (Ref::new(1), Ref::new(2), Ref::new(3));
    let mut kids = Vec::new();
    let pt = 72.0 / dpi as f32;
    for (i, path) in pages.iter().enumerate() {
        let (page_id, image_id, content_id) = (Ref::new(3 * i as i32 + 4), Ref::new(3 * i as i32 + 5), Ref::new(3 * i as i32 + 6));
        let (w, h, gray, data) = jpeg(path)?;
        let (pw, ph) = (w as f32 * pt, h as f32 * pt);
        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, pw, ph)).parent(tree).contents(content_id);
        let mut res = page.resources();
        res.x_objects().pair(Name(b"Im0"), image_id);
        res.fonts().pair(Name(b"F0"), font);
        res.finish();
        page.finish();
        let mut image = pdf.image_xobject(image_id, &data);
        image.filter(Filter::DctDecode);
        image.width(w as i32).height(h as i32).bits_per_component(8);
        if gray { image.color_space().device_gray() } else { image.color_space().device_rgb() };
        image.finish();
        let mut content = Content::new();
        content.save_state().transform([pw, 0.0, 0.0, ph, 0.0, 0.0]).x_object(Name(b"Im0")).restore_state();
        if let Some(words) = words.get(i).filter(|w| !w.is_empty()) {
            content.begin_text().set_text_rendering_mode(TextRenderingMode::Invisible);
            for word in words {
                // Helvetica's average glyph is about half the font size wide; stretch it to the box
                let size = word.h * pt;
                let text = win_ansi(&word.text);
                let natural = 0.5 * size * text.len() as f32;
                content.set_font(Name(b"F0"), size).set_horizontal_scaling(100.0 * word.w * pt / natural.max(0.01));
                // baseline a fifth of the box above its bottom edge
                content.set_text_matrix([1.0, 0.0, 0.0, 1.0, word.x * pt, ph - (word.y + 0.8 * word.h) * pt]);
                content.show(Str(&text));
            }
            content.end_text();
        }
        pdf.stream(content_id, &content.finish());
        kids.push(page_id);
    }
    pdf.type1_font(font).base_font(Name(b"Helvetica")).encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).count(kids.len() as i32).kids(kids);
    Ok(pdf.finish())
}

/// Box-average downscale of a grayscale image to `ow` x `oh`.
pub fn downscale(w: usize, h: usize, px: &[u8], ow: usize, oh: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(ow * oh);
    for y in 0..oh {
        let (y0, y1) = (y * h / oh, ((y + 1) * h / oh).max(y * h / oh + 1));
        for x in 0..ow {
            let (x0, x1) = (x * w / ow, ((x + 1) * w / ow).max(x * w / ow + 1));
            let sum: u32 = (y0..y1).flat_map(|yy| (x0..x1).map(move |xx| px[yy * w + xx] as u32)).sum();
            out.push((sum / ((y1 - y0) * (x1 - x0)) as u32) as u8);
        }
    }
    out
}

/// Saves scanned pages as one PDF, or as `out.png` / `out-1.png`, `out-2.png`... Returns the written paths.
pub fn save_scans(pages: &[String], out: &str, format: &str, dpi: u32) -> Result<Vec<String>, String> {
    if format == "OCR" {
        return ocr_pdf(pages, out, dpi);
    }
    if format == "PDF" {
        let path = format!("{out}.pdf");
        std::fs::write(&path, images_to_pdf(pages, dpi, &[])?).map_err(|e| format!("{path}: {e}"))?;
        return Ok(vec![path]);
    }
    let mut written = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let path = if pages.len() == 1 { format!("{out}.png") } else { format!("{out}-{}.png", i + 1) };
        // pages can be PNG (SANE) or JPEG (eSCL), so convert instead of copying
        save_png(&open(p)?, &path)?;
        written.push(path);
    }
    Ok(written)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A kitty graphics protocol command; inside tmux it is wrapped for passthrough.
pub fn kitty(control: &str, payload: &str, tmux: bool) -> String {
    let cmd = format!("\x1b_G{control};{payload}\x1b\\");
    if tmux { format!("\x1bPtmux;{}\x1b\\", cmd.replace('\x1b', "\x1b\x1b")) } else { cmd }
}

/// Encodes an 8-bit grayscale image buffer into a Sixel escape sequence (16 gray levels with run-length encoding).
pub fn sixel_encode(w: usize, h: usize, px: &[u8]) -> String {
    if w == 0 || h == 0 || px.len() < w * h {
        return String::new();
    }
    let mut out = format!("\x1bPq\"1;1;{w};{h}");
    for i in 0..16 {
        let pct = i * 100 / 15;
        out.push_str(&format!("#{i};2;{pct};{pct};{pct}"));
    }
    for y in (0..h).step_by(6) {
        let band_h = (h - y).min(6);
        let mut used = [false; 16];
        for dy in 0..band_h {
            let row = (y + dy) * w;
            for x in 0..w {
                used[(px[row + x] >> 4) as usize] = true;
            }
        }
        for (color, &is_used) in used.iter().enumerate() {
            if !is_used {
                continue;
            }
            out.push_str(&format!("#{color}"));
            let mut last_char = None;
            let mut count = 0;
            for x in 0..w {
                let mut mask = 0u8;
                for dy in 0..band_h {
                    if (px[(y + dy) * w + x] >> 4) == color as u8 {
                        mask |= 1 << dy;
                    }
                }
                let ch = (0x3F + mask) as char;
                if last_char == Some(ch) {
                    count += 1;
                } else {
                    if let Some(c) = last_char {
                        if count > 3 {
                            out.push_str(&format!("!{count}{c}"));
                        } else {
                            for _ in 0..count {
                                out.push(c);
                            }
                        }
                    }
                    last_char = Some(ch);
                    count = 1;
                }
            }
            if let Some(c) = last_char {
                if count > 3 {
                    out.push_str(&format!("!{count}{c}"));
                } else {
                    for _ in 0..count {
                        out.push(c);
                    }
                }
            }
            out.push('$');
        }
        out.push('-');
    }
    out.push_str("\x1b\\");
    out
}

pub const FILTERS: [&str; 3] = ["Original", "Gray", "B&W"];

/// Rotates (degrees clockwise) and filters a scanned page, always starting from the original scan.
/// Returns the edited file (the original itself when there is nothing to do) and its thumbnail.
pub fn edit_page(orig: &str, rot: u16, filter: usize) -> Result<(String, Thumb), String> {
    let out = if rot == 0 && filter == 0 {
        orig.to_string()
    } else {
        let out = format!("{orig}-r{rot}-f{filter}.png");
        let img = open(orig)?;
        let img = match rot {
            90 => img.rotate90(),
            180 => img.rotate180(),
            270 => img.rotate270(),
            _ => img,
        };
        let img = match filter {
            1 => DynamicImage::ImageLuma8(img.to_luma8()),
            2 => DynamicImage::ImageLuma8(black_and_white(img.to_luma8())),
            _ => img,
        };
        save_png(&img, &out)?;
        out
    };
    let thumb = thumbnail(&out)?;
    Ok((out, thumb))
}

#[test]
fn pages_to_pdf_and_back() {
    let dir = std::env::temp_dir().join(format!("printertui-pdf-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = |n: &str| dir.join(n).to_string_lossy().into_owned();
    // a color PNG and a gray JPEG, like SANE and eSCL scans (no file extension)
    DynamicImage::ImageRgb8(image::RgbImage::from_pixel(300, 150, image::Rgb([200, 30, 30]))).save_with_format(p("a"), ImageFormat::Png).unwrap();
    DynamicImage::ImageLuma8(GrayImage::from_pixel(150, 300, Luma([90]))).save_with_format(p("b"), ImageFormat::Jpeg).unwrap();
    let pdf = save_scans(&[p("a"), p("b")], &p("out"), "PDF", 150).unwrap().remove(0);
    assert_eq!(page_count(&pdf), Some(2));
    assert_eq!(page_count(&p("a")), None);

    let (edited, (w, h, px)) = edit_page(&p("a"), 90, 2).unwrap();
    let img = open(&edited).unwrap();
    assert_eq!((img.width(), img.height()), (150, 300));
    assert!(img.to_luma8().pixels().all(|v| v[0] == 0 || v[0] == 255));
    assert_eq!((w, px.len()), (400, w * h));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn scale_pdf_wraps_every_page() {
    let dir = std::env::temp_dir().join(format!("printertui-scale-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("p.png").to_string_lossy().into_owned();
    GrayImage::from_pixel(200, 100, Luma([0])).save(&img).unwrap();
    let pdf = dir.join("in.pdf");
    std::fs::write(&pdf, images_to_pdf(&[img.clone(), img], 100, &[]).unwrap()).unwrap();
    let out = scale_pdf(&pdf.to_string_lossy(), 50).unwrap();
    let doc = lopdf::Document::load(&out).unwrap();
    assert_eq!(doc.get_pages().len(), 2);
    for id in doc.page_iter() {
        let content = String::from_utf8_lossy(&doc.get_page_content(id)).into_owned();
        // 200x100 px at 100 dpi = 144x72 pt, centre (72, 36): moved by half of it
        assert!(content.starts_with("q 0.5000 0 0 0.5000 36.0000 18.0000 cm"), "{content}");
        assert!(content.trim_end().ends_with('Q'), "{content}");
    }
    // pdftoppm (poppler) draws it, when installed; one tool at a time, as stop_all's test kills them
    #[cfg(unix)]
    let _one = unix::SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
    if run("pdftoppm", &["-v"]).is_ok() {
        let png = dir.join("p1.png").to_string_lossy().into_owned();
        render_page(&out, 2, &png).unwrap();
        let page = open(&png).unwrap().to_luma8();
        // the black image now fills only the middle half: corners white, centre black
        assert!(page.get_pixel(1, 1)[0] > 200 && page.get_pixel(page.width() / 2, page.height() / 2)[0] < 50);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn photo_fills_the_paper_and_sheets_hold_their_pages() {
    let dir = std::env::temp_dir().join(format!("printertui-photo-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wide = dir.join("wide.png").to_string_lossy().into_owned();
    DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3000, 1000, image::Rgb([0, 0, 0]))).save(&wide).unwrap();
    let pdf = printable(&wide, 100, "A4").unwrap();
    let doc = lopdf::Document::load(&pdf).unwrap();
    let [_, _, w, h] = page_box(&doc, doc.page_iter().next().unwrap()).unwrap();
    // turned sideways it fills the A4's long side (842 pt)
    assert!((w - 842.0).abs() < 5.0 && h < 595.0, "{w} x {h}");

    let sheet = dir.join("sheet.png").to_string_lossy().into_owned();
    sheet_png(&[wide.clone(), wide.clone(), wide.clone()], "A4", 4, &sheet).unwrap();
    let img = open(&sheet).unwrap().to_luma8();
    // upright sheet, 2x2: three cells hold a black page, the bottom right one stays white
    let (sw, sh) = img.dimensions();
    assert!(sh > sw);
    assert!(img.get_pixel(sw / 4, sh / 4)[0] < 50 && img.get_pixel(sw * 3 / 4, sh / 4)[0] < 50 && img.get_pixel(sw / 4, sh * 3 / 4)[0] < 50);
    assert!(img.get_pixel(sw * 3 / 4, sh * 3 / 4)[0] > 200);
    // two to a sheet: the sheet sideways, a page on each half
    sheet_png(&[wide.clone(), wide], "A4", 2, &sheet).unwrap();
    let img = open(&sheet).unwrap().to_luma8();
    let (sw, sh) = img.dimensions();
    assert!(sw > sh && img.get_pixel(sw / 4, sh / 2)[0] < 50 && img.get_pixel(sw * 3 / 4, sh / 2)[0] < 50);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn pages_on_the_printer() {
    // A4 printable area at 600 dpi, and an A4 page (595 x 842 pt): a bit too big, so shrunk to fit
    let a4 = (4800, 6800);
    let [(x, y, w, h, turned)] = place_pages(&[(595.0, 842.0)], 1, a4, (600.0, 600.0))[..] else { panic!() };
    assert!(!turned && (4700..=4800).contains(&w) && h <= 6800 && x >= 0 && y >= 0, "{w} x {h}");
    // an A6 page prints at its own size, in the middle
    let [(x, _, w, h, _)] = place_pages(&[(298.0, 420.0)], 1, a4, (600.0, 600.0))[..] else { panic!() };
    assert!((w - 2483).abs() < 3 && (h - 3500).abs() < 3 && (x - (4800 - w) / 2).abs() <= 1, "{w} x {h}");
    // a landscape page turns on upright paper, and fills the width
    let [(_, _, w, h, turned)] = place_pages(&[(842.0, 595.0)], 1, a4, (600.0, 600.0))[..] else { panic!() };
    assert!(turned && h > w && w > 4600, "{w} x {h}");
    // 1200 dpi down and 600 across: twice the dots down for the same inches, not a squashed page
    let [(_, _, w, h, _)] = place_pages(&[(298.0, 420.0)], 1, (4800, 13600), (600.0, 1200.0))[..] else { panic!() };
    assert!((w - 2483).abs() < 3 && (h - 7000).abs() < 4, "{w} x {h}");
    // two to a sideways sheet (its area wider than tall): side by side, each filling its half
    let places = place_pages(&[(595.0, 842.0), (595.0, 842.0)], 2, (6800, 4800), (600.0, 600.0));
    assert!(places[0].0 < 3400 && places[1].0 >= 3400 && places.iter().all(|p| !p.4 && p.3 > 4700), "{places:?}");
}

#[test]
fn text_files_print_without_libreoffice() {
    let dir = std::env::temp_dir().join(format!("printertui-plaintext-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let txt = dir.join("notes.txt").to_string_lossy().into_owned();
    // 150 lines, one far longer than a line of paper: three A4 pages
    let mut text: String = (1..=150).map(|i| format!("línea {i}\n")).collect();
    text.push_str(&"x".repeat(300));
    std::fs::write(&txt, &text).unwrap();
    let pdf = printable(&txt, 100, "A4").unwrap();
    assert_eq!(page_count(&pdf), Some(3));
    let doc = lopdf::Document::load(&pdf).unwrap();
    let first = String::from_utf8_lossy(&doc.get_page_content(doc.page_iter().next().unwrap())).into_owned();
    // "línea 1" with the í in WinAnsi (0xED), as pdf-writer's hex string
    assert!(first.contains("<6CED6E65612031> Tj"), "{first}");
    // markup and binary files are left to LibreOffice
    std::fs::write(dir.join("a.html"), "<p>hi</p>").unwrap();
    assert!(plain_text(&dir.join("a.html").to_string_lossy()).is_none());
    assert!(plain_text(&pdf).is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn old_temp_files_go() {
    let tmp = std::env::temp_dir().join("printertui");
    let (old, new, sounds) = (tmp.join("00000000000000aa"), tmp.join("00000000000000bb"), tmp.join("sounds"));
    for d in [&old, &new] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("doc.pdf"), "x").unwrap();
    }
    std::fs::create_dir_all(&sounds).unwrap();
    let two_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(48 * 3600);
    std::fs::File::options().write(true).open(old.join("doc.pdf")).unwrap().set_modified(two_days_ago).unwrap();
    clean_temp();
    assert!(!old.exists() && new.exists() && sounds.exists());
    std::fs::remove_dir_all(&new).unwrap();

    let dir = std::env::temp_dir().join(format!("printertui-forget-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for f in ["page-1", "page-1-r90-f0.png", "page-1-r90-f0.png.preview.png", "page-12"] {
        std::fs::write(dir.join(f), "x").unwrap();
    }
    forget_scan(&dir.join("page-1").to_string_lossy());
    let left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    // page-12 starts with "page-1" too, but is another page
    assert_eq!(left, ["page-12"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn file_field_paths() {
    assert_eq!(split_files(" a.pdf; b.pdf ;"), ["a.pdf", "b.pdf"]);
    assert_eq!(split_files("\"C:\\Users\\Ana Pérez\\doc.pdf\""), ["C:\\Users\\Ana Pérez\\doc.pdf"]);
    assert_eq!(split_files("\"C:\\a b.pdf\" \"C:\\c.pdf\"; d.txt"), ["C:\\a b.pdf", "C:\\c.pdf", "d.txt"]);
    assert_eq!(split_files("'/home/ana/my file.pdf'"), ["/home/ana/my file.pdf"]);
    assert_eq!(split_files("Ana's notes.txt"), ["Ana's notes.txt"]);
}

#[test]
fn photos_in_more_formats() {
    let dir = std::env::temp_dir().join(format!("printertui-formats-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, format) in [("a.bmp", ImageFormat::Bmp), ("b.gif", ImageFormat::Gif), ("c.tiff", ImageFormat::Tiff), ("d.webp", ImageFormat::WebP)] {
        let path = dir.join(name).to_string_lossy().into_owned();
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(60, 40, image::Rgb([10, 20, 200]))).save_with_format(&path, format).unwrap();
        let pdf = printable(&path, 100, "A4").unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(page_count(&pdf), Some(1), "{name}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn printer_names_stay_inside_powershell_strings() {
    assert_eq!(ps_quote("HP 'Tank'"), "'HP ''Tank'''");
    // a name announced on the network that tries to end the string with a typographic quote
    assert_eq!(ps_quote("Tank\u{2019}; calc; #"), "'Tank\u{2019}\u{2019}; calc; #'");
    for q in ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'] {
        let quoted = ps_quote(&format!("a{q}b{q}{q}c"));
        let inner: String = quoted.chars().skip(1).take(quoted.chars().count() - 2).collect();
        // inside, quotes only come in pairs, which PowerShell reads as one quote and not as the end
        assert!(inner.split(|c: char| c != q).all(|run| run.chars().count() % 2 == 0), "{quoted}");
    }
    assert_eq!(ps_encoded("A"), base64(&[0x41, 0]));
    assert!(check_uri("ipp://192.168.1.46:631/ipp/print").is_ok());
    assert!(check_uri("ipps://HP4A8B2C.local/ipp/print").is_ok());
    assert!(check_uri("ipp://x/ipp/print'; calc").is_err());
    assert!(check_uri("ipp://x/ipp print").is_err());
    assert!(check_uri("file:///C:/Windows").is_err());
}

#[test]
fn failing_tools_say_why() {
    #[cfg(unix)]
    let _one = unix::SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(unix)]
    let err = run("sh", &["-c", "exit 3"]).unwrap_err();
    #[cfg(windows)]
    let err = run("cmd", &["/C", "exit 3"]).unwrap_err();
    assert!(err.contains('3'), "{err}");
}

#[test]
fn config_with_a_byte_order_mark() {
    assert_eq!(parse_config("\u{feff}printer=Tank\r\nlang=es\n"), [("printer", "Tank"), ("lang", "es")]);
}

#[test]
fn ipp_answer_to_state() {
    // a Get-Printer-Attributes answer: status ok, printer group, then the attributes
    let mut data = vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04];
    let mut attr = |tag: u8, name: &str, value: &[u8]| {
        data.push(tag);
        data.extend((name.len() as u16).to_be_bytes());
        data.extend(name.as_bytes());
        data.extend((value.len() as u16).to_be_bytes());
        data.extend(value);
    };
    attr(0x23, "printer-state", &3i32.to_be_bytes());
    attr(0x44, "printer-state-reasons", b"media-empty-error");
    attr(0x44, "", b"marker-supply-low-warning");
    attr(0x42, "marker-colors", b"#00FFFF");
    attr(0x42, "", b"#000000");
    attr(0x21, "marker-levels", &8i32.to_be_bytes());
    attr(0x21, "", &60i32.to_be_bytes());
    attr(0x21, "marker-low-levels", &2i32.to_be_bytes());
    attr(0x21, "", &2i32.to_be_bytes());
    data.push(0x03);
    let state = PrinterState::from(&parse_ipp(&data).unwrap());
    assert_eq!(state.ink, [(0x00FFFF, 8, true), (0, 60, false)]);
    assert_eq!(state.problems, ["media-empty"]);
    // an error status, or a cut-off answer, is no answer
    assert!(parse_ipp(&[1, 1, 0x04, 0x06, 0, 0, 0, 1, 3]).is_none());
    assert!(parse_ipp(&data[..30]).is_none());
}

#[test]
fn black_and_white_keeps_text_on_pale_paper() {
    // pale gray paper (200) with darker gray text (150): a fixed 60% cut would blacken the paper
    let img = GrayImage::from_fn(100, 100, |x, _| Luma([if x < 10 { 150 } else { 200 }]));
    let bw = black_and_white(img);
    assert_eq!(bw.get_pixel(5, 5)[0], 0);
    assert_eq!(bw.get_pixel(50, 5)[0], 255);
}

#[test]
fn searchable_pdf_text_layer() {
    let dir = std::env::temp_dir().join(format!("printertui-text-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("p").to_string_lossy().into_owned();
    DynamicImage::ImageLuma8(GrayImage::from_pixel(1200, 300, Luma([255]))).save_with_format(&img, ImageFormat::Png).unwrap();
    let words = vec![Word { text: "Hola".into(), x: 50.0, y: 100.0, w: 200.0, h: 60.0 }, Word { text: "cañón".into(), x: 300.0, y: 100.0, w: 250.0, h: 60.0 }];
    let pdf = dir.join("out.pdf");
    std::fs::write(&pdf, images_to_pdf(&[img], 150, &[words]).unwrap()).unwrap();
    assert_eq!(page_count(&pdf.to_string_lossy()), Some(1));
    // pdftotext (poppler) reads the text back, when installed
    if let Ok(text) = run("pdftotext", &["-enc", "UTF-8", &pdf.to_string_lossy(), "-"]) {
        assert!(text.contains("Hola") && text.contains("cañón"), "{text}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_sixel_encode() {
    let px = vec![0u8; 12 * 12];
    let six = sixel_encode(12, 12, &px);
    assert!(six.starts_with("\x1bPq\"1;1;12;12"));
    assert!(six.ends_with("\x1b\\"));
    assert!(six.contains("#0"));
}
