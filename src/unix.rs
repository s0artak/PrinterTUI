//! Linux and macOS: CUPS command line tools (lp, lpstat, lpinfo, lpadmin) and SANE.

use crate::*;
use std::process::Command;

pub const ADD_PRINTER_NOTE: &str = "sudo may ask for your password";
pub const PICK_HINT: &str = "install yazi, lf, ranger, nnn or fzf";
pub const SCANNER_HINT: &str = "for other scanners install SANE";

/// Printer URI schemes that point at a network printer.
const NETWORK: [&str; 6] = ["ipp", "ipps", "socket", "lpd", "http", "https"];

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

/// Display name for a queue from its `lpoptions -p` output: the description if the user set one
/// (`lpadmin -p QUEUE -D "Name"`), else the model, plus the network address.
pub fn printer_label(queue: &str, lpoptions: &str) -> String {
    let opt = |key| lpoption(lpoptions, key);
    let info = opt("printer-info").filter(|i| !i.is_empty() && i != queue);
    let model = opt("printer-make-and-model").map(|m| m.trim_end_matches(" - IPP Everywhere").to_string());
    let name = info.or(model).unwrap_or_else(|| queue.to_string());
    match opt("device-uri").as_deref().and_then(uri_host) {
        Some((scheme, host)) if NETWORK.contains(&scheme) => format!("{name} ({host})"),
        _ => name,
    }
}

/// One value from `lpoptions -p` output: key=value, key='quoted value' or key=escaped\ value.
fn lpoption(lpoptions: &str, key: &str) -> Option<String> {
    let v = lpoptions.split(&format!(" {key}=")).nth(1).or_else(|| lpoptions.strip_prefix(&format!("{key}=")))?;
    let v = match v.strip_prefix('\'') {
        Some(q) => q.split('\'').next()?,
        None => v.split(' ').next()?,
    };
    Some(v.replace("\\ ", " "))
}

/// Ink or toner left as (RGB color, percent, -1 when unknown) from the printer's IPP marker
/// attributes; empty when it does not report them.
pub fn ink(queue: &str) -> Vec<(u32, i32)> {
    parse_ink(&run("lpoptions", &["-p", queue]).unwrap_or_default())
}

pub fn parse_ink(lpoptions: &str) -> Vec<(u32, i32)> {
    let (Some(colors), Some(levels)) = (lpoption(lpoptions, "marker-colors"), lpoption(lpoptions, "marker-levels")) else {
        return Vec::new();
    };
    colors
        .split(',')
        .zip(levels.split(','))
        // a marker with several colors ("#00FFFF#FF00FF") shows the first
        .map(|(c, l)| (c.get(1..7).and_then(|c| u32::from_str_radix(c, 16).ok()).unwrap_or(0x808080), l.parse().ok().filter(|l| *l >= 0).unwrap_or(-1)))
        .collect()
}

pub fn printer_labels(queues: &[String]) -> Vec<String> {
    queues.iter().map(|q| printer_label(q, &run("lpoptions", &["-p", q]).unwrap_or_default())).collect()
}

/// Sends a job with `lp`, returns its "request id is ..." line.
pub fn submit(job: &Job) -> Result<String, String> {
    let args = lp_args(job);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run("lp", &args)?;
    if let Some(id) = job_id(&out) {
        JOBS.lock().unwrap().push(id.to_string());
    }
    Ok(out)
}

/// True while the job is still pending, held or printing.
pub fn job_active(id: &str) -> bool {
    run("lpstat", &["-W", "not-completed", "-o"])
        .is_ok_and(|s| s.lines().any(|l| l.split_whitespace().next() == Some(id)))
}

/// Unfinished jobs on all printers as (job number, "file  size  position") for the queue popup.
pub fn queue() -> Vec<(String, String)> {
    parse_lpq(&run("lpq", &["-a"]).unwrap_or_default())
}

/// `lpq -a` rows: "1st  spartak  8  my file.txt  1024 bytes" (the header and "no entries" are skipped).
pub fn parse_lpq(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let [rank, owner, job, ref file @ .., size, "bytes"] = w[..] else { return None };
            job.parse::<u32>().ok()?;
            Some((job.to_string(), format!("{}  ({owner}, {} KB, {rank})", file.join(" "), size.parse::<u64>().unwrap_or(0).div_ceil(1024))))
        })
        .collect()
}

pub fn cancel_job(id: &str) -> Result<(), String> {
    run("cancel", &[id]).map(drop)
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

fn to_ipp(uri: &str) -> Option<(String, String)> {
    let (scheme, host) = uri_host(uri)?;
    let name = queue_name(host);
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


pub fn lp_args(job: &Job) -> Vec<String> {
    let mut a = vec![
        "-d".into(), job.printer.clone(),
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
    a.extend(["--".into(), job.file.clone()]);
    a
}

/// One page of a PDF as a PNG for the preview: poppler's pdftoppm, or on macOS without it the
/// system's sips, which only draws the first page.
pub fn render_page(pdf: &str, page: u32, png: &str) -> Result<(), String> {
    let n = page.to_string();
    let prefix = png.trim_end_matches(".png");
    match run("pdftoppm", &["-f", &n, "-l", &n, "-r", "100", "-png", "-singlefile", pdf, prefix]) {
        Err(_) if cfg!(target_os = "macos") && page == 1 => run("sips", &["-s", "format", "png", pdf, "--out", png]).map(drop),
        Err(e) if cfg!(target_os = "macos") => Err(format!("Previewing pages after the first needs poppler (brew install poppler): {e}")),
        r => r.map(drop).map_err(|e| format!("Preview needs pdftoppm (poppler): {e}")),
    }
}

/// Hosts of the network printers in CUPS, to try as eSCL scanners.
pub fn printer_hosts() -> Vec<String> {
    run("lpstat", &["-v"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| uri_host(l.rsplit(' ').next()?))
        .filter(|(scheme, _)| NETWORK.contains(scheme))
        .map(|(_, host)| host.to_string())
        .collect()
}

/// SANE devices as (device, "vendor model").
pub fn local_scanners() -> Vec<(String, String)> {
    let sane = run("scanimage", &["-f", "%d\t%v %m%n"]).unwrap_or_default();
    sane.lines().filter_map(|l| l.split_once('\t')).map(|(d, v)| (d.to_string(), v.to_string())).collect()
}

pub fn scan_local(device: &str, mode: &str, dpi: u32, out: &str) -> Result<(), String> {
    let dpi = dpi.to_string();
    run("scanimage", &["-d", device, "--mode", mode, "--resolution", &dpi, "--format=png", "-o", out]).map(drop)
}

/// Stops running tools, their children first (e.g. LibreOffice's soffice.bin) so none are left orphaned.
pub fn kill_tree(pids: &[u32]) {
    let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
    let _ = Command::new("pkill").args(["-TERM", "-P", &pids.join(",")]).status();
    let _ = Command::new("kill").arg("-TERM").args(&pids).status();
}

/// LibreOffice; macOS does not put it on the PATH.
pub fn soffice() -> String {
    if cfg!(target_os = "macos") { "/Applications/LibreOffice.app/Contents/MacOS/soffice" } else { "libreoffice" }.into()
}

/// Local time for file names: 2026-09-26_154200.
pub fn timestamp() -> String {
    run("date", &["+%Y-%m-%d_%H%M%S"]).unwrap_or_default()
}

/// Searchable PDF: tesseract lays the recognised text invisibly over each page image.
pub fn ocr_pdf(pages: &[String], out: &str, dpi: u32) -> Result<Vec<String>, String> {
    let langs = run("tesseract", &["--list-langs"]).map_err(|_| "Searchable PDF needs tesseract (and tesseract-data-<language>)")?;
    // ponytail: every installed language except osd; slower with many installed, add a picker then
    let langs: Vec<&str> = langs.lines().skip(1).filter(|l| *l != "osd").collect();
    let tmp = std::env::temp_dir().join(format!("printertui-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    // JPEG copies keep the PDF small (tesseract embeds the images as they are), and a list file
    // with one image per line makes tesseract write all pages into one PDF
    let res = pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let jpg = tmp.join(format!("{i}.jpg")).to_string_lossy().into_owned();
            std::fs::write(&jpg, jpeg(p)?.3).map_err(|e| e.to_string()).map(|_| jpg)
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|jpgs| {
            let list = tmp.join("pages.txt");
            std::fs::write(&list, jpgs.join("\n")).map_err(|e| e.to_string())?;
            run("tesseract", &[&list.to_string_lossy(), out, "--dpi", &dpi.to_string(), "-l", &langs.join("+"), "pdf"])
        });
    let _ = std::fs::remove_dir_all(&tmp);
    res.map(|_| vec![format!("{out}.pdf")])
}

#[test]
fn ink_levels() {
    let out = "device-uri=ipp://192.168.1.46/ipp/print marker-colors=#00FFFF,#000000 marker-levels=80,-1 marker-names='cyan\\ ink,black\\ ink' printer-info=Tank";
    assert_eq!(parse_ink(out), [(0x00FFFF, 80), (0, -1)]);
    assert_eq!(printer_label("q", out), "Tank (192.168.1.46)");
    assert!(parse_ink("printer-info=x").is_empty());
}

#[test]
fn lpq_rows() {
    let out = "Rank    Owner   Job     File(s)                         Total Size\n\
               active  ana     12      my report.pdf                   20480 bytes\n\
               no entries";
    assert_eq!(parse_lpq(out), [("12".to_string(), "my report.pdf  (ana, 20 KB, active)".to_string())]);
}

/// Tests that spawn tools run one at a time, since `stop_all` kills every running tool.
#[cfg(test)]
static SPAWNS: Mutex<()> = Mutex::new(());

#[test]
fn stop_all_kills_tools_and_their_children() {
    let _one = SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
    let t = std::thread::spawn(|| run("sh", &["-c", "sleep 30 & wait"]));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let start = std::time::Instant::now();
    stop_all();
    let _ = t.join();
    assert!(start.elapsed().as_secs() < 5);
    assert!(Command::new("pgrep").args(["-f", "^sleep 30$"]).output().unwrap().stdout.is_empty());
}

#[test]
fn ocr_pdf_has_text() {
    let _one = SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
    // the test page is drawn with ImageMagick, which the app itself no longer needs
    if run("tesseract", &["--version"]).is_err() || run("magick", &["--version"]).is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("printertui-ocr-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (img, out) = (dir.join("p.png").to_string_lossy().into_owned(), dir.join("out").to_string_lossy().into_owned());
    run("magick", &["-size", "1200x300", "xc:white", "-pointsize", "72", "-annotate", "+50+180", "Hello printer", &img]).unwrap();
    let pdf = save_scans(&[img], &out, "OCR", 150).unwrap().remove(0);
    let text = run("pdftotext", &[&pdf, "-"]).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(text.contains("Hello printer"), "{text}");
}

