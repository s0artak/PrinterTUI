//! Thin wrappers around the CUPS command line tools (lp, lpstat, lpinfo, lpadmin).

use std::process::Command;

pub const PAPERS: [&str; 5] = ["A4", "Letter", "Legal", "A5", "A3"];
/// Pages per sheet side (`number-up`).
pub const PER_SHEET: [u32; 6] = [1, 2, 4, 6, 9, 16];

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Configured printers, default printer first.
pub fn printers() -> Vec<String> {
    let mut list: Vec<String> = run("lpstat", &["-e"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    if let Some(def) = run("lpstat", &["-d"])
        .ok()
        .and_then(|s| s.rsplit(": ").next().map(str::to_string))
    {
        if let Some(i) = list.iter().position(|p| *p == def) {
            list.swap(0, i);
        }
    }
    list
}

/// Page count of a PDF (qpdf ships with cups-filters).
pub fn page_count(file: &str) -> Option<u32> {
    run("qpdf", &["--show-npages", file]).ok()?.parse().ok()
}

/// Converts a document, text or image to PDF with LibreOffice, returns the PDF path.
pub fn to_pdf(file: &str) -> Result<String, String> {
    let dir = std::env::temp_dir().join("printertui");
    let d = dir.to_str().ok_or("Bad temp dir")?;
    let stem = std::path::Path::new(file).file_stem().ok_or("Bad file name")?;
    let pdf = format!("{d}/{}.pdf", stem.to_string_lossy());
    let _ = std::fs::remove_file(&pdf);
    // own profile, so a running LibreOffice window does not swallow the conversion
    let profile = format!("-env:UserInstallation=file://{d}/profile");
    run("libreoffice", &[&profile, "--headless", "--convert-to", "pdf", "--outdir", d, file])
        .map_err(|e| format!("Could not convert {file} to PDF (is libreoffice installed?): {e}"))?;
    page_count(&pdf).map(|_| pdf).ok_or(format!("LibreOffice could not convert {file} to PDF"))
}

/// Paths in the File field are separated by ';'.
pub fn split_files(field: &str) -> Vec<String> {
    field.split(';').map(str::trim).filter(|f| !f.is_empty()).map(String::from).collect()
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

pub struct Job<'a> {
    pub printer: &'a str,
    pub file: &'a str,
    pub color: bool,
    pub paper: &'a str,
    pub pages: Option<String>,
    pub reverse: bool,
    pub copies: u32,
    pub collate: bool,
    pub per_sheet: u32,
}

pub fn lp_args(job: &Job) -> Vec<String> {
    let mut a = vec![
        "-d".into(), job.printer.into(),
        "-o".into(), format!("media={}", job.paper),
        "-o".into(), format!("print-color-mode={}", if job.color { "color" } else { "monochrome" }),
    ];
    if let Some(p) = &job.pages {
        a.extend(["-o".into(), format!("page-ranges={p}")]);
    }
    if job.reverse {
        a.extend(["-o".into(), "outputorder=reverse".into()]);
    }
    if job.copies > 1 {
        a.extend(["-n".into(), job.copies.to_string(), "-o".into(), format!("collate={}", job.collate)]);
    }
    if job.per_sheet > 1 {
        a.extend(["-o".into(), format!("number-up={}", job.per_sheet)]);
    }
    a.extend(["--".into(), job.file.into()]);
    a
}

/// Settings file: $XDG_CONFIG_HOME/printertui/config, else ~/.config/printertui/config.
pub fn config_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    Some(base.join("printertui/config"))
}

/// `key=value` lines as trimmed pairs; other lines are skipped.
pub fn parse_config(text: &str) -> Vec<(&str, &str)> {
    text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect()
}

/// Sends a job with `lp`, returns its "request id is ..." line.
pub fn submit(args: &[String]) -> Result<String, String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run("lp", &args)
}

/// "request id is P-12 (1 file(s))" -> "P-12".
pub fn job_id(lp_out: &str) -> Option<&str> {
    lp_out.strip_prefix("request id is ")?.split_whitespace().next()
}

/// True while the job is still pending, held or printing.
pub fn job_active(id: &str) -> bool {
    run("lpstat", &["-W", "not-completed", "-o"])
        .is_ok_and(|s| s.lines().any(|l| l.split_whitespace().next() == Some(id)))
}

/// Network printers found by `lpinfo -v`, as (queue name, IPP uri) ready for `lpadmin -m everywhere`.
pub fn discover() -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = run("lpinfo", &["-v"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter_map(to_ipp)
        .collect();
    found.sort();
    found.dedup_by(|a, b| a.0 == b.0);
    found
}

/// ("ipp", "192.168.1.46") from "ipp://192.168.1.46/ipp/print".
pub fn uri_host(uri: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = uri.split_once("://")?;
    Some((scheme, rest.split(['/', ':', '?']).next().filter(|h| !h.is_empty())?))
}

fn to_ipp(uri: &str) -> Option<(String, String)> {
    let (scheme, host) = uri_host(uri)?;
    let name = format!("printer_{}", host.replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
    match scheme {
        "ipp" | "ipps" | "dnssd" => Some((name, uri.to_string())),
        // ponytail: assumes the standard IPP Everywhere path; edit the queue with lpadmin if a printer differs
        "socket" | "lpd" => Some((name, format!("ipp://{host}/ipp/print"))),
        _ => None,
    }
}

/// Interactive: sudo may ask for a password, so the terminal must be in normal mode.
pub fn add_printer(name: &str, uri: &str) -> Result<(), String> {
    let ok = Command::new("sudo")
        .args(["lpadmin", "-p", name, "-E", "-v", uri, "-m", "everywhere"])
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if ok { Ok(()) } else { Err(format!("lpadmin failed for {uri}")) }
}

/// Opens the first installed terminal file manager as a picker, returns the selected files.
/// Needs the terminal in normal mode.
pub fn pick_files() -> Vec<String> {
    let out = std::env::temp_dir().join(format!("printertui-pick-{}", std::process::id()));
    let o = out.to_str().unwrap_or_default();
    let pickers: [(&str, Vec<String>); 4] = [
        ("yazi", vec![format!("--chooser-file={o}")]),
        ("lf", vec!["-selection-path".into(), o.into()]),
        ("ranger", vec![format!("--choosefiles={o}")]),
        ("nnn", vec!["-p".into(), o.into()]),
    ];
    for (cmd, args) in pickers {
        if Command::new(cmd).args(&args).status().is_ok() {
            let picked = std::fs::read_to_string(&out).unwrap_or_default();
            let _ = std::fs::remove_file(&out);
            return lines(&picked);
        }
    }
    // fzf draws on the tty and prints the choices on stdout
    Command::new("fzf").arg("-m").stdout(std::process::Stdio::piped()).output()
        .map_or(Vec::new(), |out| lines(&String::from_utf8_lossy(&out.stdout)))
}

/// One path per line (nnn may separate them with NUL).
fn lines(s: &str) -> Vec<String> {
    s.split(['\n', '\0']).filter(|l| !l.is_empty()).map(String::from).collect()
}

pub const SCAN_MODES: [&str; 2] = ["Color", "Gray"];
pub const SCAN_DPI: [u32; 3] = [150, 300, 600];

/// Scanners as (SANE device, description). Network printers already in CUPS are tried first
/// as eSCL scanners by address, which is instant and reliable; SANE's own discovery (slow,
/// finds USB scanners too) runs when `full` is set or nothing was found that way.
pub fn scanners(full: bool) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = run("lpstat", &["-v"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| uri_host(l.rsplit(' ').next()?))
        .filter(|(scheme, _)| ["ipp", "ipps", "socket", "lpd", "http", "https"].contains(scheme))
        .filter_map(|(_, host)| {
            let caps = run("curl", &["-sf", "-m", "2", &format!("http://{host}/eSCL/ScannerCapabilities")]).ok()?;
            let model = caps.split("<pwg:MakeAndModel>").nth(1)?.split('<').next()?.to_string();
            Some((format!("airscan:escl:{model}:http://{host}/eSCL"), model))
        })
        .collect();
    if full || found.is_empty() {
        let sane = run("scanimage", &["-f", "%d\t%v %m%n"]).unwrap_or_default();
        for (d, v) in sane.lines().filter_map(|l| l.split_once('\t')) {
            // the same device found by address already is the reliable entry
            if !found.iter().any(|(_, model)| v.contains(model.as_str())) {
                found.push((d.to_string(), v.to_string()));
            }
        }
    }
    found
}

/// Scans one page from the flatbed into a PNG.
pub fn scan(device: &str, mode: &str, dpi: u32, out: &str) -> Result<(), String> {
    let dpi = dpi.to_string();
    run("scanimage", &["-d", device, "--mode", mode, "--resolution", &dpi, "--format=png", "-o", out]).map(drop)
}

/// Grayscale thumbnail of an image as (width, height, pixels), via ImageMagick.
pub fn thumbnail(image: &str) -> Result<(usize, usize, Vec<u8>), String> {
    let pgm = format!("{image}.pgm");
    // thicken text before shrinking, or it fades to near-white at terminal resolution
    run("magick", &[image, "-colorspace", "Gray", "-morphology", "Erode", "Disk:2", "-resize", "400x", &pgm])?;
    let bytes = std::fs::read(&pgm).map_err(|e| e.to_string())?;
    parse_pgm(&bytes).ok_or_else(|| "Could not read the scan preview".into())
}

/// Parses a binary 8-bit PGM (P5) as written by ImageMagick.
pub fn parse_pgm(b: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 {
        while b.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while !b.get(i)?.is_ascii_whitespace() {
            i += 1;
        }
        fields.push(std::str::from_utf8(&b[start..i]).ok()?);
    }
    let (w, h): (usize, usize) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
    let px = b.get(i + 1..i + 1 + w * h)?;
    (fields[0] == "P5" && fields[3] == "255").then(|| (w, h, px.to_vec()))
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
pub fn save_scans(pages: &[String], out: &str, pdf: bool, dpi: u32) -> Result<Vec<String>, String> {
    if pdf {
        let path = format!("{out}.pdf");
        // the density sets the PDF page size (pixels / dpi), otherwise ImageMagick assumes 72 dpi
        let dpi = dpi.to_string();
        let mut args = vec!["-units", "PixelsPerInch", "-density", &dpi];
        args.extend(pages.iter().map(String::as_str));
        args.extend(["-compress", "jpeg", "-quality", "85"]);
        args.push(&path);
        run("magick", &args)?;
        return Ok(vec![path]);
    }
    let mut written = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let path = if pages.len() == 1 { format!("{out}.png") } else { format!("{out}-{}.png", i + 1) };
        std::fs::copy(p, &path).map_err(|e| format!("{path}: {e}"))?;
        written.push(path);
    }
    Ok(written)
}
