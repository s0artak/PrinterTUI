//! Linux and macOS: CUPS command line tools (lp, lpstat, lpinfo, lpadmin) and SANE.

use crate::*;
use std::process::Command;


/// The user's language from the locale: "es_ES.UTF-8", "C" when unset.
pub fn system_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.is_empty()).unwrap_or_default()
}

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

/// The printer's ink and problems: asked over IPP when CUPS reaches it that way (live, and
/// "offline" when it does not answer), else CUPS' copy from its last job.
pub fn printer_state(queue: &str) -> PrinterState {
    let opts = run("lpoptions", &["-p", queue]).unwrap_or_default();
    let uri = lpoption(&opts, "device-uri").unwrap_or_default();
    // a printer macOS added has a Bonjour name: its address, to ask it directly
    let uri = if uri.starts_with("dnssd://") { bonjour_uri(&uri).unwrap_or(uri) } else { uri };
    if !matches!(uri_host(&uri), Some(("ipp" | "ipps" | "http", _))) {
        return cups_state(&opts);
    }
    ipp_attributes(&uri, None, &STATE_ATTRIBUTES).map_or_else(
        |_| {
            let mut state = cups_state(&opts);
            state.problems.push("offline".into());
            state
        },
        |attrs| PrinterState::from(&attrs),
    )
}

/// CUPS keeps the printer's state attributes as queue options: marker-levels=80,100.
fn cups_state(lpoptions: &str) -> PrinterState {
    let attrs = STATE_ATTRIBUTES
        .iter()
        .filter_map(|k| Some((k.to_string(), lpoption(lpoptions, k)?.split(',').map(|v| IppValue::Text(v.to_string())).collect())))
        .collect();
    PrinterState::from(&attrs)
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

/// A job's IPP state (3 pending ... 5 printing, 7 canceled, 8 aborted, 9 completed) and the sheet
/// sides printed so far, as CUPS hears them from the printer; only the state if CUPS cannot be asked.
pub fn job_progress(id: &str) -> (i32, u32) {
    let ask = || {
        let (queue, n) = id.rsplit_once('-')?;
        let attrs = ipp_attributes(&format!("ipp://localhost/printers/{queue}"), Some(n.parse().ok()?), &["job-state", "job-impressions-completed"]).ok()?;
        let int = |k| match attrs.get(k)?.first()? {
            IppValue::Int(v) => Some(*v),
            IppValue::Text(_) => None,
        };
        Some((int("job-state")?, int("job-impressions-completed").unwrap_or(0).max(0) as u32))
    };
    ask().unwrap_or_else(|| (if job_active(id) { 5 } else { 9 }, 0))
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

/// macOS: the system's file dialog (cancelling it picks nothing).
#[cfg(target_os = "macos")]
pub fn pick_files() -> Vec<String> {
    let script = [
        "activate",
        "set picked to choose file with prompt \"PrinterTUI\" with multiple selections allowed",
        "set out to \"\"",
        "repeat with f in picked",
        "set out to out & POSIX path of f & linefeed",
        "end repeat",
        "return out",
    ];
    let args: Vec<&str> = script.iter().flat_map(|l| ["-e", *l]).collect();
    run("osascript", &args).map_or(Vec::new(), |out| lines(&out))
}

/// Opens the first installed terminal file manager as a picker, returns the selected files.
/// Needs the terminal in normal mode.
#[cfg(not(target_os = "macos"))]
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

/// macOS: JavaScript for Automation, which reaches the system's own frameworks (PDFKit, Vision)
/// with nothing to install. `run(argv)` gets `args`; what it returns is the output.
#[cfg(target_os = "macos")]
fn jxa(script: &str, args: &[&str]) -> Result<String, String> {
    let mut all = vec!["-l", "JavaScript", "-e", script];
    all.extend(args);
    run("osascript", &all)
}

/// PDFKit draws page argv[1] of argv[0] at 100 dpi into the PNG argv[2].
#[cfg(target_os = "macos")]
const RENDER_JS: &str = r#"
ObjC.import('PDFKit'); ObjC.import('AppKit');
function run(argv) {
  const doc = $.PDFDocument.alloc.initWithURL($.NSURL.fileURLWithPath(argv[0]));
  if (doc.isNil()) throw new Error('cannot open ' + argv[0]);
  const page = doc.pageAtIndex(Number(argv[1]) - 1);
  if (page.isNil()) throw new Error('no page ' + argv[1]);
  const box = page.boundsForBox(0);
  let w = box.size.width * 100 / 72, h = box.size.height * 100 / 72;
  if (page.rotation % 180 != 0) [w, h] = [h, w];
  const img = page.thumbnailOfSizeForBox($.NSMakeSize(Math.round(w), Math.round(h)), 0);
  const png = $.NSBitmapImageRep.imageRepWithData(img.TIFFRepresentation).representationUsingTypeProperties(4, $({}));
  if (!png.writeToFileAtomically(argv[2], true)) throw new Error('cannot write ' + argv[2]);
}"#;

/// Vision reads the text of each image in argv: one line per text line found, as
/// "image index, x, y, width, height, text" separated by tabs, the box in 0..1 from the bottom left.
#[cfg(target_os = "macos")]
const OCR_JS: &str = r#"
ObjC.import('Vision'); ObjC.import('Foundation');
function run(argv) {
  const out = [];
  argv.forEach((path, n) => {
    const req = $.VNRecognizeTextRequest.alloc.init;
    req.recognitionLevel = 0;
    req.usesLanguageCorrection = true;
    if (req.respondsToSelector('setAutomaticallyDetectsLanguage:')) req.automaticallyDetectsLanguage = true;
    const handler = $.VNImageRequestHandler.alloc.initWithURLOptions($.NSURL.fileURLWithPath(path), $({}));
    if (!handler.performRequestsError($([req]), null)) throw new Error('Vision could not read ' + path);
    const res = req.results;
    for (let i = 0; i < res.count; i++) {
      const obs = res.objectAtIndex(i);
      const top = obs.topCandidates(1);
      if (top.count == 0) continue;
      const b = obs.boundingBox;
      out.push([n, b.origin.x, b.origin.y, b.size.width, b.size.height, top.objectAtIndex(0).string.js.replace(/[\t\n]/g, ' ')].join('\t'));
    }
  });
  return out.join('\n');
}"#;

/// macOS: a photo in a format only the system reads (iPhone's HEIC, HEIF, AVIF, WebP, TIFF...)
/// as a JPEG, converted by sips; None for other files.
pub fn photo_to_jpeg(file: &str) -> Option<Result<String, String>> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let ext = std::path::Path::new(file).extension()?.to_string_lossy().to_lowercase();
    if !matches!(ext.as_str(), "heic" | "heif" | "hif" | "avif" | "webp" | "tif" | "tiff" | "gif" | "bmp") {
        return None;
    }
    Some(work_dir(file).and_then(|dir| {
        // sips can leave an empty file behind (seen on virtual Macs), so the result is checked,
        // and made again as a PNG if it cannot be read
        // under Rosetta (an Intel build on Apple silicon) sips would run as Intel code too, and
        // then cannot decode HEIC: run the native one
        let rosetta = run("sysctl", &["-n", "sysctl.proc_translated"]).is_ok_and(|v| v == "1");
        let sips: &[&str] = if rosetta { &["arch", "-arm64", "sips"] } else { &["sips"] };
        let mut last = String::new();
        for (format, name) in [("jpeg", "photo.jpg"), ("png", "photo.png")] {
            let out = dir.join(name).to_string_lossy().into_owned();
            let args: Vec<&str> = sips[1..].iter().copied().chain(["-s", "format", format, file, "--out", &out]).collect();
            match run(sips[0], &args).and_then(|_| image_size(&out)) {
                Ok(_) => return Ok(out),
                Err(e) => last = e,
            }
        }
        Err(format!("{file}: macOS could not convert the photo ({last})"))
    }))
}

/// One page of a PDF as a PNG for the preview: macOS draws it itself (PDFKit), elsewhere
/// poppler's pdftoppm, which comes with CUPS.
pub fn render_page(pdf: &str, page: u32, png: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return jxa(RENDER_JS, &[pdf, &page.to_string(), png]).map(drop).map_err(|e| format!("Preview: {e}"));
    #[cfg(not(target_os = "macos"))]
    {
        let n = page.to_string();
        let prefix = png.trim_end_matches(".png");
        run("pdftoppm", &["-f", &n, "-l", &n, "-r", "100", "-png", "-singlefile", pdf, prefix])
            .map(drop)
            .map_err(|e| format!("Preview needs pdftoppm (poppler): {e}"))
    }
}

/// Hosts of the network printers in CUPS, to try as eSCL scanners.
pub fn printer_hosts() -> Vec<String> {
    let mut hosts: Vec<String> = run("lpstat", &["-v"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let uri = l.rsplit(' ').next()?;
            // printers macOS adds itself have a Bonjour name instead of an address
            let uri = if uri.starts_with("dnssd://") { bonjour_uri(uri)? } else { uri.to_string() };
            let (scheme, host) = uri_host(&uri)?;
            NETWORK.contains(&scheme).then(|| host.to_string())
        })
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// Bonjour names already looked up, and the printer address each has.
static BONJOUR: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// The printer address (ipp://HP4A8B2C.local:631/ipp/print) of a Bonjour one
/// (dnssd://Name._ipps._tcp.local./?uuid=...), as macOS adds printers: CUPS' ippfind looks up
/// every printer on the network at once, and the answers are kept.
fn bonjour_uri(uri: &str) -> Option<String> {
    let name = dnssd_name(uri)?;
    let known = |list: &[(String, String)]| list.iter().find(|(n, _)| *n == name).map(|(_, u)| u.clone());
    if let Some(found) = known(&BONJOUR.lock().unwrap()) {
        return Some(found);
    }
    let answer = run("ippfind", &["-T", "3", "_ipp._tcp,local.", "_ipps._tcp,local.", "--exec", "echo", "{service_name}|||{service_uri}", ";"]);
    let mut list = BONJOUR.lock().unwrap();
    list.extend(bonjour_answers(&answer.unwrap_or_default()));
    known(&list)
}

/// The Bonjour service name in a dnssd:// printer address, with its %20-style escapes decoded.
fn dnssd_name(uri: &str) -> Option<String> {
    let service = uri.strip_prefix("dnssd://")?.split(['/', '?']).next()?;
    let name = &service[..service.find("._ipp")?];
    let bytes = name.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], name.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// ippfind's "name|||uri" lines as (name, uri).
fn bonjour_answers(found: &str) -> Vec<(String, String)> {
    found
        .lines()
        .filter_map(|l| l.split_once("|||"))
        .filter(|(_, uri)| uri.contains("://"))
        .map(|(name, uri)| (name.to_string(), uri.trim().to_string()))
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

/// Searchable PDF: on macOS the system's text recognition (Vision), else tesseract, lays the
/// recognised text invisibly over each page image.
pub fn ocr_pdf(pages: &[String], out: &str, dpi: u32) -> Result<Vec<String>, String> {
    #[cfg(target_os = "macos")]
    if let Ok(words) = vision_words(pages) {
        let path = format!("{out}.pdf");
        std::fs::write(&path, images_to_pdf(pages, dpi, &words)?).map_err(|e| format!("{path}: {e}"))?;
        return Ok(vec![path]);
    }
    tesseract_pdf(pages, out, dpi)
}

/// The text lines Vision finds on each page, with their boxes in the image's pixels.
#[cfg(target_os = "macos")]
fn vision_words(pages: &[String]) -> Result<Vec<Vec<Word>>, String> {
    let args: Vec<&str> = pages.iter().map(String::as_str).collect();
    let found = jxa(OCR_JS, &args)?;
    let sizes = pages.iter().map(|p| image_size(p)).collect::<Result<Vec<_>, _>>()?;
    let mut words: Vec<Vec<Word>> = pages.iter().map(|_| Vec::new()).collect();
    for line in found.lines() {
        let f: Vec<&str> = line.splitn(6, '\t').collect();
        let [n, x, y, w, h, text] = f[..] else { continue };
        let (Ok(n), Ok(x), Ok(y), Ok(w), Ok(h)) = (n.parse::<usize>(), x.parse::<f32>(), y.parse::<f32>(), w.parse::<f32>(), h.parse::<f32>()) else { continue };
        let Some(&(iw, ih)) = sizes.get(n) else { continue };
        let (iw, ih) = (iw as f32, ih as f32);
        // Vision's boxes start at the bottom left; images at the top left
        words[n].push(Word { text: text.to_string(), x: x * iw, y: (1.0 - y - h) * ih, w: w * iw, h: h * ih });
    }
    Ok(words)
}

fn tesseract_pdf(pages: &[String], out: &str, dpi: u32) -> Result<Vec<String>, String> {
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
fn cups_copy_of_the_state() {
    let out = "device-uri=usb://x marker-colors=#00FFFF,#000000 marker-levels=80,-1 marker-low-levels=2,2 \
               marker-names='cyan\\ ink,black\\ ink' printer-info=Tank printer-state=5 printer-state-reasons=media-jam-error";
    let state = cups_state(out);
    assert_eq!(state.ink, [(0x00FFFF, 80, false), (0, -1, false)]);
    assert_eq!(state.problems, ["media-jam", "stopped"]);
    assert_eq!(printer_label("q", out), "Tank");
    assert_eq!(cups_state("printer-info=x"), PrinterState::default());
}

/// PDFKit draws a page, and Vision reads back the text on it.
#[cfg(target_os = "macos")]
#[test]
fn macos_draws_and_reads_pages() {
    let _one = SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("printertui-macos-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let txt = dir.join("hello.txt").to_string_lossy().into_owned();
    std::fs::write(&txt, "HELLO PRINTER\n\nsecond page follows\n").unwrap();
    let pdf = printable(&txt, 100, "A4").unwrap();
    let png = dir.join("page.png").to_string_lossy().into_owned();
    render_page(&pdf, 1, &png).unwrap();
    let (w, h) = image_size(&png).unwrap();
    // A4 at 100 dpi
    assert!((820..=830).contains(&w) && (1165..=1175).contains(&h), "{w}x{h}");
    let words = vision_words(&[png.clone()]).unwrap();
    let text: Vec<&str> = words[0].iter().map(|w| w.text.as_str()).collect();
    assert!(text.iter().any(|t| t.contains("HELLO PRINTER")), "{text:?}");
    // the box is near the top left of the page, where the text is
    let first = words[0].iter().find(|w| w.text.contains("HELLO")).unwrap();
    assert!(first.x < w as f32 / 3.0 && first.y < h as f32 / 5.0, "{} {}", first.x, first.y);
    let searchable = ocr_pdf(&[png], &dir.join("out").to_string_lossy(), 100).unwrap().remove(0);
    assert_eq!(page_count(&searchable), Some(1));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// An iPhone photo (HEIC) prints as a photo.
#[cfg(target_os = "macos")]
#[test]
fn macos_prints_heic_photos() {
    let _one = SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("printertui-heic-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("photo.png").to_string_lossy().into_owned();
    image::RgbImage::from_fn(400, 300, |x, _| image::Rgb([(x % 256) as u8, 90, 200])).save(&png).unwrap();
    let heic = dir.join("IMG_0001.HEIC").to_string_lossy().into_owned();
    run("sips", &["-s", "format", "heic", &png, "--out", &heic]).unwrap();
    let pdf = printable(&heic, 100, "A4").unwrap();
    assert_eq!(page_count(&pdf), Some(1));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Printers macOS adds itself: their Bonjour name, and the address ippfind finds for it.
#[test]
fn bonjour_printers() {
    let uri = "dnssd://HP%20Smart%20Tank%205100%20series%20%5B4A8B2C%5D._ipps._tcp.local./?uuid=1234";
    assert_eq!(dnssd_name(uri).as_deref(), Some("HP Smart Tank 5100 series [4A8B2C]"));
    assert_eq!(dnssd_name("ipp://192.168.1.46/ipp/print"), None);
    let found = "HP Smart Tank 5100 series [4A8B2C]|||ipps://HP4A8B2C.local:631/ipp/print\nbroken line\nx|||nothing";
    let answers = bonjour_answers(found);
    assert_eq!(answers, [("HP Smart Tank 5100 series [4A8B2C]".to_string(), "ipps://HP4A8B2C.local:631/ipp/print".to_string())]);
    assert_eq!(uri_host(&answers[0].1), Some(("ipps", "HP4A8B2C.local")));
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
pub(crate) static SPAWNS: Mutex<()> = Mutex::new(());

#[test]
fn stop_all_kills_tools_and_their_children() {
    let _one = SPAWNS.lock().unwrap_or_else(|e| e.into_inner());
    let t = std::thread::spawn(|| run("sh", &["-c", "sleep 30 & wait"]));
    // until the shell is running and has started its sleep (slow machines take a while)
    let sleeping = || !Command::new("pgrep").args(["-f", "^sleep 30$"]).output().unwrap().stdout.is_empty();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while (crate::CHILDREN.lock().unwrap().is_empty() || !sleeping()) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
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

