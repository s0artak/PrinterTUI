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
    let mut list: Vec<String> = run_c("lpstat", &["-e"]).unwrap_or_default().lines().map(str::to_string).collect();
    if let Some(def) = run_c("lpstat", &["-d"]).ok().and_then(|s| s.rsplit(": ").next().map(str::to_string))
        && let Some(i) = list.iter().position(|p| *p == def)
    {
        list.swap(0, i);
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
    let uri = opt("device-uri").unwrap_or_default();
    match uri_host(&uri) {
        // a Bonjour service name is no address to show
        Some((scheme, host)) if NETWORK.contains(&scheme) && service_name(&uri).is_none() => format!("{name} ({host})"),
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
    // a printer known by its Bonjour name (macOS adds them as dnssd://, CUPS sets up the ones it
    // finds as ipps://Name._ipps._tcp.local./): its address, to ask it directly
    let uri = if service_name(&uri).is_some() { bonjour_uri(&uri).unwrap_or(uri) } else { uri };
    // a name that cannot be looked up says nothing about the printer itself
    let reachable =
        uri_host(&uri).is_some_and(|(scheme, host)| matches!(scheme, "ipp" | "ipps" | "http") && service_name(&uri).is_none() && resolve_host(host).is_some());
    if !reachable {
        return cups_state(&opts);
    }
    ipp_attributes(&uri, &STATE_ATTRIBUTES).map_or_else(
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

/// Names Avahi announces (HP4A8B2C.local) as addresses. The Linux release is a static binary whose
/// resolver only knows DNS and /etc/hosts, so it asks the system's own tools, which go through
/// nss-mdns: glibc's getent, else Avahi. The answers are kept. Other names, and every name on
/// macOS (its resolver knows Bonjour), are left as they are; None when a .local name is not found.
pub fn resolve_host(host: &str) -> Option<String> {
    static KNOWN: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let name = host.trim_end_matches('.');
    if cfg!(target_os = "macos") || !name.to_ascii_lowercase().ends_with(".local") {
        return Some(host.to_string());
    }
    if let Some((_, ip)) = KNOWN.lock().unwrap().iter().find(|(n, _)| n == name) {
        return Some(ip.clone());
    }
    let ipv4 = |s: &str| s.parse::<std::net::Ipv4Addr>().is_ok().then(|| s.to_string());
    // getent: "192.168.1.46  STREAM HP4A8B2C.local"; avahi: "HP4A8B2C.local\t192.168.1.46"
    let ip = run("getent", &["ahostsv4", name])
        .ok()
        .and_then(|out| out.split_whitespace().next().and_then(ipv4))
        .or_else(|| run("avahi-resolve-host-name", &["-4", name]).ok().and_then(|out| out.split_whitespace().nth(1).and_then(ipv4)))?;
    KNOWN.lock().unwrap().push((name.to_string(), ip.clone()));
    Some(ip)
}

/// The paper the queue is set up for (CUPS' media default), else the system's (/etc/papersize,
/// LC_PAPER), as one of PAPERS: where to start before one is chosen.
pub fn default_paper(queue: &str) -> Option<&'static str> {
    let media = lpoption(&run("lpoptions", &["-p", queue]).unwrap_or_default(), "media");
    let system = || {
        std::fs::read_to_string("/etc/papersize").ok().map(|s| s.trim().to_string()).or_else(|| {
            // "height=279 width=216" is Letter, "height=297 width=210" A4
            let paper = run("locale", &["-k", "LC_PAPER"]).ok()?;
            match paper.lines().find_map(|l| l.strip_prefix("width="))? {
                "216" => Some("letter".into()),
                "210" => Some("a4".into()),
                _ => None,
            }
        })
    };
    media.or_else(system).and_then(|m| paper_name(&m))
}

/// One of PAPERS for a media name: "na_letter_8.5x11in", "iso_a4_210x297mm", "Letter", "a4".
fn paper_name(media: &str) -> Option<&'static str> {
    let m = media.to_ascii_lowercase();
    PAPERS.iter().copied().find(|p| {
        let p = p.to_ascii_lowercase();
        m == p || m.starts_with(&format!("na_{p}_")) || m.starts_with(&format!("iso_{p}_"))
    })
}

/// Sends a job with `lp`, returns its "request id is ..." line.
pub fn submit(job: &Job) -> Result<String, String> {
    let args = lp_args(job);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run("lp", &args)
}

/// True while the job is still pending, held or printing.
pub fn job_active(id: &str) -> bool {
    run_c("lpstat", &["-W", "not-completed", "-o"]).is_ok_and(|s| s.lines().any(|l| l.split_whitespace().next() == Some(id)))
}

/// Unfinished jobs on all printers as (job number, "file  size  position") for the queue popup.
/// lpq names the files; where it is missing (Debian puts it in cups-bsd), lpstat lists the jobs.
pub fn queue() -> Vec<(String, String)> {
    match run_c("lpq", &["-a"]) {
        Ok(text) => parse_lpq(&text),
        Err(_) => parse_lpstat_jobs(&run_c("lpstat", &["-W", "not-completed", "-o"]).unwrap_or_default()),
    }
}

/// `lpq -a` rows: "1st  spartak  8  my file.txt  1024 bytes" (the header and "no entries" are skipped).
pub fn parse_lpq(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let [rank, owner, job, ref file @ .., size, _unit] = w[..] else { return None };
            job.parse::<u32>().ok()?;
            let size = size.parse::<u64>().ok()?;
            Some((job.to_string(), format!("{}  ({owner}, {} KB, {rank})", file.join(" "), size.div_ceil(1024))))
        })
        .collect()
}

/// `lpstat -o` rows: "HP_Tank-12  ana  20480  Sat 04 Oct 2026 10:00:00 AM CEST".
fn parse_lpstat_jobs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            let [job, owner, size, ..] = w[..] else { return None };
            let size = size.parse::<u64>().ok()?;
            Some((job.to_string(), format!("{job}  ({owner}, {} KB)", size.div_ceil(1024))))
        })
        .collect()
}

pub fn cancel_job(id: &str) -> Result<(), String> {
    run("cancel", &[id]).map(drop)
}

/// A CUPS admin tool: on the PATH, else in /usr/sbin, which Debian and openSUSE keep off a user's PATH.
fn tool(cmd: &str) -> String {
    let on_path = std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(cmd).is_file()));
    if on_path { cmd.into() } else { format!("/usr/sbin/{cmd}") }
}

/// Network printers found by `lpinfo -v` that have no queue yet, as (queue name, IPP uri) ready
/// for `lpadmin -m everywhere`. CUPS lets only its admin group list devices; its refusal is the error.
pub fn discover() -> Result<Vec<(String, String)>, String> {
    let schemes = "dnssd,ipp,ipps,lpd,snmp,socket";
    let devices = run_c(&tool("lpinfo"), &["--timeout", "5", "--include-schemes", schemes, "-v"])?;
    let queued = run_c("lpstat", &["-v"]).unwrap_or_default();
    let mut found: Vec<(String, String)> = devices
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|uri| !queued.lines().any(|q| q.ends_with(&format!(" {uri}"))))
        .filter_map(to_ipp)
        .collect();
    found.sort();
    found.dedup_by(|a, b| a.0 == b.0);
    Ok(found)
}

fn to_ipp(uri: &str) -> Option<(String, String)> {
    let (scheme, host) = uri_host(uri)?;
    // a Bonjour service makes a readable queue name: HP_Smart_Tank_5100_series_4A8B2C
    let name = match service_name(uri) {
        Some(service) => service.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("_"),
        None => queue_name(host),
    };
    match scheme {
        "ipp" | "ipps" | "dnssd" => Some((name, uri.to_string())),
        // ponytail: assumes the standard IPP Everywhere path; edit the queue with lpadmin if a printer differs
        "socket" | "lpd" => Some((name, format!("ipp://{host}/ipp/print"))),
        _ => None,
    }
}

/// Adds a printer with CUPS' driverless IPP Everywhere setup. Members of CUPS' admin group
/// (lpadmin on Debian and Ubuntu, wheel or sys elsewhere) need no password; anyone else is asked
/// for it by sudo (or doas, run0, pkexec), so the terminal must be in normal mode.
pub fn add_printer(name: &str, uri: &str) -> Result<(), String> {
    let lpadmin = tool("lpadmin");
    let args = ["-p", name, "-E", "-v", uri, "-m", "everywhere"];
    match run_c(&lpadmin, &args) {
        Ok(_) => return Ok(()),
        Err(e) if !e.contains("Forbidden") && !e.contains("Unauthorized") && !e.contains("not authorized") => return Err(e),
        Err(_) => {}
    }
    let on_path = |cmd: &str| std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(cmd).is_file()));
    let helper =
        ["sudo", "doas", "run0", "pkexec"].into_iter().find(|h| on_path(h)).ok_or("Adding a printer needs administrator rights, and there is no sudo")?;
    // the password is asked on the terminal; lpadmin's own message is kept for the status
    let out = Command::new(helper).arg(&lpadmin).args(args).stderr(std::process::Stdio::piped()).output().map_err(|e| format!("{helper}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if why.is_empty() { format!("lpadmin failed for {uri}") } else { why })
}

/// macOS: the system's file dialog (cancelling it picks nothing).
#[cfg(target_os = "macos")]
pub fn pick_files() -> Option<Vec<String>> {
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
    Some(run("osascript", &args).map_or(Vec::new(), |out| lines(&out)))
}

/// Linux: the files picked, None when there is nothing to pick them with. On a desktop (not over
/// SSH) its own dialog: KDE's kdialog, else GNOME's zenity; then the first terminal file manager
/// installed, then fzf. Needs the terminal in normal mode. Cancelling picks nothing.
#[cfg(not(target_os = "macos"))]
pub fn pick_files() -> Option<Vec<String>> {
    use std::process::Stdio;
    let var = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    let docs = documents_dir().filter(|d| d.is_dir()).or_else(std::env::home_dir).unwrap_or_else(|| ".".into());
    // a dialog's exit code 1 is a cancel; anything it cannot start falls through to the next one
    let dialog = |cmd: &str, args: &[String]| -> Option<Vec<String>> {
        let out = Command::new(cmd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        match out.status.code() {
            Some(0) => Some(lines(&String::from_utf8_lossy(&out.stdout))),
            Some(1) => Some(Vec::new()),
            _ => None,
        }
    };
    if (var("WAYLAND_DISPLAY") || var("DISPLAY")) && !var("SSH_CONNECTION") && !var("SSH_TTY") {
        let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("KDE"));
        let kdialog = || dialog("kdialog", &["--getopenfilename".into(), docs.to_string_lossy().into(), "--multiple".into(), "--separate-output".into()]);
        let zenity = || {
            let start = format!("{}/", docs.to_string_lossy());
            dialog(
                "zenity",
                &["--file-selection".into(), "--multiple".into(), "--separator=\n".into(), "--title=PrinterTUI".into(), format!("--filename={start}")],
            )
        };
        let picked = if kde { kdialog().or_else(zenity) } else { zenity().or_else(kdialog) };
        if picked.is_some() {
            return picked;
        }
    }
    let out = temp_root().join(format!("pick-{}", std::process::id()));
    let o = out.to_str().unwrap_or_default();
    let pickers: [(&str, Vec<String>); 4] = [
        ("yazi", vec![format!("--chooser-file={o}")]),
        ("lf", vec!["-selection-path".into(), o.into()]),
        ("ranger", vec![format!("--choosefiles={o}")]),
        ("nnn", vec!["-p".into(), o.into()]),
    ];
    for (cmd, args) in pickers {
        if Command::new(cmd).args(&args).current_dir(&docs).status().is_ok() {
            let picked = std::fs::read_to_string(&out).unwrap_or_default();
            let _ = std::fs::remove_file(&out);
            return Some(lines(&picked));
        }
    }
    // fzf lists the files below the folder it runs in, draws on the terminal (stderr) and prints
    // the choices on stdout
    let fzf = Command::new("fzf").arg("-m").current_dir(&docs).stdin(Stdio::inherit()).stderr(Stdio::inherit()).stdout(Stdio::piped()).output().ok()?;
    Some(lines(&String::from_utf8_lossy(&fzf.stdout)).into_iter().map(|f| docs.join(f).to_string_lossy().into_owned()).collect())
}

/// One path per line (nnn may separate them with NUL).
fn lines(s: &str) -> Vec<String> {
    s.split(['\n', '\0']).filter(|l| !l.is_empty()).map(String::from).collect()
}

pub fn lp_args(job: &Job) -> Vec<String> {
    let mut a = vec![
        "-d".into(),
        job.printer.clone(),
        "-o".into(),
        format!("media={}", job.paper),
        "-o".into(),
        format!("print-color-mode={}", if job.color { "color" } else { "monochrome" }),
        // one side, whatever the queue's default: double-sided is done by turning the stack
        "-o".into(),
        "sides=one-sided".into(),
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

/// macOS: a photo in a format only the system reads (iPhone's HEIC, HEIF, AVIF...) as a JPEG,
/// converted by sips; None for other files.
pub fn convert_photo(file: &str) -> Option<Result<String, String>> {
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

/// Hosts of the network printers in CUPS, to try as eSCL scanners, as addresses this app can reach.
pub fn printer_hosts() -> Vec<String> {
    let mut hosts: Vec<(String, String)> = run_c("lpstat", &["-v"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let uri = l.rsplit(' ').next()?;
            // printers known by a Bonjour name instead of an address
            let uri = if service_name(uri).is_some() { bonjour_uri(uri)? } else { uri.to_string() };
            let (scheme, host) = uri_host(&uri)?;
            if !NETWORK.contains(&scheme) {
                return None;
            }
            Some((resolve_host(host)?, host.to_string()))
        })
        .collect();
    // one per printer, by its name rather than its address when a queue has it, so a scanner
    // saved as the one to use is still that one when the network gives the printer a new address
    hosts.sort_by_key(|(ip, host)| (ip.clone(), host.parse::<std::net::IpAddr>().is_ok()));
    hosts.dedup_by(|a, b| a.0 == b.0);
    hosts.into_iter().map(|(_, host)| host).collect()
}

/// Bonjour names already looked up, and the printer address each has.
static BONJOUR: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// The printer address (ipp://HP4A8B2C.local:631/ipp/print) of a Bonjour one
/// (dnssd://Name._ipps._tcp.local./?uuid=...), as macOS adds printers: CUPS' ippfind looks up
/// every printer on the network at once, and the answers are kept.
fn bonjour_uri(uri: &str) -> Option<String> {
    let name = service_name(uri)?;
    let known = |list: &[(String, String)]| list.iter().find(|(n, _)| *n == name).map(|(_, u)| u.clone());
    if let Some(found) = known(&BONJOUR.lock().unwrap()) {
        return Some(found);
    }
    let answer = run("ippfind", &["-T", "3", "_ipp._tcp,local.", "_ipps._tcp,local.", "--exec", "echo", "{service_name}|||{service_uri}", ";"]);
    let mut list = BONJOUR.lock().unwrap();
    list.extend(bonjour_answers(&answer.unwrap_or_default()));
    known(&list)
}

/// The Bonjour service name in a printer address that has one instead of a host, with its
/// %20-style escapes decoded: dnssd://Name._ipps._tcp.local./?uuid=... as macOS adds printers,
/// ipps://Name._ipps._tcp.local./ as CUPS sets up the printers it finds itself.
fn service_name(uri: &str) -> Option<String> {
    let (scheme, rest) = uri.split_once("://")?;
    if !matches!(scheme, "dnssd" | "ipp" | "ipps") {
        return None;
    }
    let service = rest.split(['/', '?']).next()?;
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

/// Scans with SANE's scanimage. Most backends call the modes Color and Gray; some (Brother's
/// brscan: "24bit Color", "True Gray") do not, or lack the resolution asked for: then the device's
/// own choices (`scanimage -A`) are read and the scan is tried once more with the nearest ones.
pub fn scan_local(device: &str, mode: &str, dpi: u32, out: &str) -> Result<(), String> {
    let scan = |mode: Option<&str>, dpi: u32| {
        let dpi = dpi.to_string();
        let mut args = vec!["-d", device, "--resolution", &dpi, "--format=png", "-o", out];
        if let Some(mode) = mode {
            args.extend(["--mode", mode]);
        }
        run("scanimage", &args).map(drop)
    };
    match scan(Some(mode), dpi) {
        Err(e) if e.contains("--mode") || e.contains("--resolution") || e.contains("Invalid argument") => {
            let (modes, resolutions, range) = sane_options(&run("scanimage", &["-d", device, "-A"]).unwrap_or_default());
            // stopped while the choices were asked for: no second scan
            if scan_cancelled() {
                return Err(SCAN_STOPPED.into());
            }
            let mode = sane_mode(&modes, mode);
            let dpi = match (resolutions.iter().min_by_key(|r| r.abs_diff(dpi)), range) {
                (Some(&r), _) => r,
                (None, Some((lo, hi))) => dpi.clamp(lo, hi),
                (None, None) => dpi,
            };
            scan(mode.as_deref(), dpi).map_err(|_| e)
        }
        res => res,
    }
}

/// A device's --mode choices, --resolution list and --resolution range from `scanimage -A`:
/// "    --mode Lineart|Gray|Color [Color]", "    --resolution 75|150|300dpi [150]",
/// "    --resolution 50..2400dpi (in steps of 1) [150]".
fn sane_options(help: &str) -> (Vec<String>, Vec<u32>, Option<(u32, u32)>) {
    let value = |line: &str, option: &str| -> Option<String> {
        let v = line.trim().strip_prefix(option)?.strip_prefix(' ')?;
        // the default in brackets at the end, "[Color]", is not a choice ("Gray[Error Diffusion]" is)
        let v = match v.rfind(" [") {
            Some(i) if v.ends_with(']') => &v[..i],
            _ => v,
        };
        Some(v.split(" (").next().unwrap_or(v).trim().to_string())
    };
    let modes = help.lines().find_map(|l| value(l, "--mode")).map_or(Vec::new(), |v| v.split('|').map(|m| m.trim().to_string()).collect());
    let res = help.lines().find_map(|l| value(l, "--resolution")).unwrap_or_default();
    let res = res.trim_end_matches("dpi");
    let range = res.split_once("..").and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().trim_end_matches("dpi").parse().ok()?)));
    let list = if range.is_some() { Vec::new() } else { res.split('|').filter_map(|r| r.trim().trim_end_matches("dpi").parse().ok()).collect() };
    (modes, list, range)
}

/// The device's own name for Color or Gray: an exact match, else a color mode that is not the
/// fast (lower quality) one, else a gray that is not dithered; None when it has no --mode.
fn sane_mode(modes: &[String], mode: &str) -> Option<String> {
    if modes.is_empty() {
        return None;
    }
    let lower: Vec<String> = modes.iter().map(|m| m.to_lowercase()).collect();
    let pick = |ok: &dyn Fn(&str) -> bool| lower.iter().position(|m| ok(m)).map(|i| modes[i].clone());
    if mode == "Gray" {
        pick(&|m| m == "gray")
            .or_else(|| pick(&|m| m == "true gray"))
            .or_else(|| pick(&|m| (m.starts_with("gray") || m.starts_with("grey")) && !m.contains('[')))
            .or_else(|| pick(&|m| m.contains("gray") || m.contains("grey")))
    } else {
        pick(&|m| m == "color").or_else(|| pick(&|m| m.contains("color") && !m.contains("fast"))).or_else(|| pick(&|m| m.contains("color")))
    }
}

/// Stops running tools, their children first (e.g. LibreOffice's soffice.bin) so none are left
/// orphaned. A running scanimage first gets the interrupt it answers by cancelling the scan (TERM
/// would leave the scanner mid-scan), and up to 2 seconds to do it.
pub fn kill_tree(pids: &[u32]) {
    let list: Vec<String> = pids.iter().map(u32::to_string).collect();
    let ps = |fields: &str, pids: &[String]| {
        Command::new("ps").args(["-o", fields, "-p", &pids.join(",")]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
    };
    let scans: Vec<String> = ps("pid=,comm=", &list)
        .lines()
        // ps pads the pids on the left
        .filter_map(|l| l.trim_start().split_once(char::is_whitespace))
        .filter(|(_, comm)| comm.trim().ends_with("scanimage"))
        .map(|(pid, _)| pid.trim().to_string())
        .collect();
    if !scans.is_empty() {
        let _ = Command::new("kill").arg("-INT").args(&scans).stderr(std::process::Stdio::null()).status();
        // gone, or a zombie its thread has not collected yet
        let running = || ps("stat=", &scans).lines().any(|s| !s.trim().is_empty() && !s.trim().starts_with('Z'));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while running() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    // quietly: some have ended by now
    let quiet = |cmd: &mut Command| {
        let _ = cmd.stderr(std::process::Stdio::null()).status();
    };
    quiet(Command::new("pkill").args(["-TERM", "-P", &list.join(",")]));
    quiet(Command::new("kill").arg("-TERM").args(&list));
}

/// The Documents folder: ~/Documents on macOS; on Linux the one in ~/.config/user-dirs.dirs,
/// which desktops name in their language (~/Documentos, ~/Dokumente...).
pub fn documents_dir() -> Option<std::path::PathBuf> {
    let home = std::env::home_dir()?;
    #[cfg(not(target_os = "macos"))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map_or_else(|| home.join(".config"), std::path::PathBuf::from);
        if let Some(docs) = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|dirs| xdg_documents(&dirs, &home)) {
            return Some(docs);
        }
    }
    Some(home.join("Documents"))
}

/// XDG_DOCUMENTS_DIR="$HOME/Documentos" in user-dirs.dirs as a path; "$HOME/" alone turns it off.
#[cfg(any(not(target_os = "macos"), test))]
fn xdg_documents(dirs: &str, home: &std::path::Path) -> Option<std::path::PathBuf> {
    let value = dirs.lines().find_map(|l| l.trim().strip_prefix("XDG_DOCUMENTS_DIR="))?.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => std::path::PathBuf::from(value),
    };
    (path != home).then_some(path)
}

/// LibreOffice: the system's, the one from libreoffice.org in /opt, or Flathub's or the Snap
/// Store's, which see only their own folders. macOS does not put it on the PATH.
pub fn soffice() -> Office {
    let plain = |cmd: String| Office { cmd, pre: Vec::new(), sandbox: None };
    if cfg!(target_os = "macos") {
        return plain("/Applications/LibreOffice.app/Contents/MacOS/soffice".into());
    }
    let home = std::env::home_dir().unwrap_or_default();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let on_path = |cmd: &str| std::env::split_paths(&path).map(|d| d.join(cmd)).find(|p| p.is_file());
    if let Some(found) = on_path("libreoffice").or_else(|| on_path("soffice")) {
        // the snap's sees $HOME but not /tmp, which it has its own of
        let snap = found.starts_with("/snap") || found.starts_with("/var/lib/snapd/snap");
        let sandbox = snap.then(|| home.join("snap/libreoffice/common/printertui"));
        return Office { cmd: found.to_string_lossy().into_owned(), pre: Vec::new(), sandbox };
    }
    // /opt/libreoffice25.8: the newest when there are several
    let opt = std::fs::read_dir("/opt").into_iter().flatten().flatten().filter_map(|e| {
        let version = e.file_name().to_string_lossy().strip_prefix("libreoffice")?.split('.').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
        Some((version, e.path().join("program/soffice")))
    });
    if let Some((_, soffice)) = opt.filter(|(_, p)| p.is_file()).max() {
        return plain(soffice.to_string_lossy().into_owned());
    }
    const FLATPAK: &str = "org.libreoffice.LibreOffice";
    let installed = [std::path::PathBuf::from("/var/lib/flatpak/app"), home.join(".local/share/flatpak/app")].iter().any(|d| d.join(FLATPAK).is_dir());
    if installed && on_path("flatpak").is_some() {
        return Office {
            cmd: "flatpak".into(),
            pre: vec!["run".into(), FLATPAK.into()],
            sandbox: Some(home.join(".var/app").join(FLATPAK).join("cache/printertui")),
        };
    }
    // not installed: the error names it
    plain("libreoffice".into())
}

/// A real folder of this user's, not a link or someone else's.
pub fn own_dir(path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: getuid has no preconditions and cannot fail
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.uid() == unsafe { libc::getuid() })
}

/// /tmp/printertui-<uid>, made by this user and open only to them; a folder or link someone
/// else put there first is not used, but the user's cache folder instead.
pub fn private_temp() -> std::path::PathBuf {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    // SAFETY: getuid has no preconditions and cannot fail
    let uid = unsafe { libc::getuid() };
    let dir = std::env::temp_dir().join(format!("printertui-{uid}"));
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
    if let Ok(m) = std::fs::symlink_metadata(&dir)
        && m.is_dir()
        && m.uid() == uid
    {
        if m.mode() & 0o077 != 0 {
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        return dir;
    }
    let home = std::env::home_dir().unwrap_or_default();
    let cache = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()).map_or_else(|| home.join(".cache"), std::path::PathBuf::from);
    let dir = cache.join("printertui");
    let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
    dir
}

/// Local time for file names: 2026-09-26_154200.
pub fn timestamp() -> String {
    run("date", &["+%Y-%m-%d_%H%M%S"]).unwrap_or_default()
}

/// Searchable PDF: on macOS the system's text recognition (Vision), else tesseract, lays the
/// recognised text invisibly over each page image.
pub fn ocr_pdf(pages: &[(String, u32)], out: &str) -> Result<Vec<String>, String> {
    #[cfg(target_os = "macos")]
    if let Ok(words) = vision_words(&pages.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()) {
        let path = format!("{out}.pdf");
        std::fs::write(&path, pages_to_pdf(pages, &words)?).map_err(|e| format!("{path}: {e}"))?;
        return Ok(vec![path]);
    }
    tesseract_pdf(pages, out)
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
        let (Ok(n), Ok(x), Ok(y), Ok(w), Ok(h)) = (n.parse::<usize>(), x.parse::<f32>(), y.parse::<f32>(), w.parse::<f32>(), h.parse::<f32>()) else {
            continue;
        };
        let Some(&(iw, ih)) = sizes.get(n) else { continue };
        let (iw, ih) = (iw as f32, ih as f32);
        // Vision's boxes start at the bottom left; images at the top left
        words[n].push(Word { text: text.to_string(), x: x * iw, y: (1.0 - y - h) * ih, w: w * iw, h: h * ih });
    }
    Ok(words)
}

fn tesseract_pdf(pages: &[(String, u32)], out: &str) -> Result<Vec<String>, String> {
    let langs = run("tesseract", &["--list-langs"]).map_err(|_| "Searchable PDF needs tesseract (and tesseract-data-<language>)")?;
    // ponytail: every installed language except osd; slower with many installed, add a picker then
    let langs: Vec<&str> = langs.lines().skip(1).filter(|l| *l != "osd").collect();
    let tmp = temp_root().join(format!("ocr-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    // JPEG copies keep the PDF small (tesseract embeds the images as they are), and a list file
    // with one image per line makes tesseract write all pages into one PDF; each JPEG says its
    // own resolution, which tesseract sizes its page by
    let res = pages
        .iter()
        .enumerate()
        .map(|(i, (p, dpi))| {
            let jpg = tmp.join(format!("{i}.jpg")).to_string_lossy().into_owned();
            std::fs::write(&jpg, with_jpeg_dpi(jpeg(p)?.3, *dpi)).map_err(|e| e.to_string()).map(|_| jpg)
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|jpgs| {
            let list = tmp.join("pages.txt");
            std::fs::write(&list, jpgs.join("\n")).map_err(|e| e.to_string())?;
            run("tesseract", &[&list.to_string_lossy(), out, "-l", &langs.join("+"), "pdf"])
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
    let words = vision_words(std::slice::from_ref(&png)).unwrap();
    let text: Vec<&str> = words[0].iter().map(|w| w.text.as_str()).collect();
    assert!(text.iter().any(|t| t.contains("HELLO PRINTER")), "{text:?}");
    // the box is near the top left of the page, where the text is
    let first = words[0].iter().find(|w| w.text.contains("HELLO")).unwrap();
    assert!(first.x < w as f32 / 3.0 && first.y < h as f32 / 5.0, "{} {}", first.x, first.y);
    let searchable = ocr_pdf(&[(png, 100)], &dir.join("out").to_string_lossy()).unwrap().remove(0);
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
    assert_eq!(service_name(uri).as_deref(), Some("HP Smart Tank 5100 series [4A8B2C]"));
    assert_eq!(service_name("ipp://192.168.1.46/ipp/print"), None);
    let found = "HP Smart Tank 5100 series [4A8B2C]|||ipps://HP4A8B2C.local:631/ipp/print\nbroken line\nx|||nothing";
    let answers = bonjour_answers(found);
    assert_eq!(answers, [("HP Smart Tank 5100 series [4A8B2C]".to_string(), "ipps://HP4A8B2C.local:631/ipp/print".to_string())]);
    assert_eq!(uri_host(&answers[0].1), Some(("ipps", "HP4A8B2C.local")));
}

#[test]
fn documents_folder_in_the_desktop_language() {
    let home = std::path::Path::new("/home/ana");
    let dirs = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Escritorio\"\nXDG_DOCUMENTS_DIR=\"$HOME/Documentos\"\n";
    assert_eq!(xdg_documents(dirs, home), Some(home.join("Documentos")));
    assert_eq!(xdg_documents("XDG_DOCUMENTS_DIR=\"/srv/docs\"", home), Some("/srv/docs".into()));
    assert_eq!(xdg_documents("XDG_DOCUMENTS_DIR=\"$HOME/\"", home), None);
    assert_eq!(xdg_documents("", home), None);
}

#[test]
fn jobs_and_options_cups_sees() {
    // lpq's size unit is translated (octets, Bytes, байт); the number is what counts
    let fr = "Rang    Propriétaire Tâche    Fichier(s)       Taille totale\nactive  ana     12      rapport.pdf      20480 octets";
    assert_eq!(parse_lpq(fr), [("12".to_string(), "rapport.pdf  (ana, 20 KB, active)".to_string())]);
    // where lpq is missing, lpstat lists the jobs
    let jobs = "HP_Tank-12              ana          20480   Sat 04 Oct 2026 10:00:00 AM CEST";
    assert_eq!(parse_lpstat_jobs(jobs), [("HP_Tank-12".to_string(), "HP_Tank-12  (ana, 20 KB)".to_string())]);
    // one side, whatever the queue's default
    let job = Job { printer: "Q".into(), file: "f.pdf".into(), color: false, paper: "A4", pages: None, reverse: false, copies: 1, collate: true, per_sheet: 1 };
    assert!(lp_args(&job).windows(2).any(|w| w == ["-o", "sides=one-sided"]));
    // the paper a queue or the system is set up for
    assert_eq!(paper_name("na_letter_8.5x11in"), Some("Letter"));
    assert_eq!(paper_name("iso_a4_210x297mm"), Some("A4"));
    assert_eq!(paper_name("letter"), Some("Letter"));
    assert_eq!(paper_name("na_legal_8.5x14in"), Some("Legal"));
    assert_eq!(paper_name("om_small-photo_100x150mm"), None);
    // names that are not .local are left to the resolver
    assert_eq!(resolve_host("192.168.1.46").as_deref(), Some("192.168.1.46"));
}

#[test]
fn printers_cups_sets_up_by_itself() {
    // a driverless printer CUPS found: its device-uri holds the Bonjour service name, not a host
    let uri = "ipps://HP%20Smart%20Tank%205100%20series%20%5B4A8B2C%5D._ipps._tcp.local./";
    assert_eq!(service_name(uri).as_deref(), Some("HP Smart Tank 5100 series [4A8B2C]"));
    let opts = format!("device-uri={uri} printer-make-and-model='HP\\ Smart\\ Tank\\ 5100'");
    assert_eq!(printer_label("HP_Smart_Tank", &opts), "HP Smart Tank 5100");
    assert_eq!(printer_label("q", "device-uri=ipp://192.168.1.46/ipp/print printer-info=Tank"), "Tank (192.168.1.46)");
    // adding one found by Bonjour gives a readable queue name
    let dnssd = "dnssd://HP%20Smart%20Tank%205100%20series%20%5B4A8B2C%5D._ipp._tcp.local./?uuid=1";
    assert_eq!(to_ipp(dnssd).map(|(name, _)| name).as_deref(), Some("HP_Smart_Tank_5100_series_4A8B2C"));
    assert_eq!(to_ipp("socket://192.168.1.46"), Some(("printer_192_168_1_46".into(), "ipp://192.168.1.46/ipp/print".into())));
}

#[test]
fn sane_choices_of_each_backend() {
    let pixma = "    --mode Color|Gray|Lineart [Color]\n        Selects the scan mode.\n    --resolution 75|150|300|600|1200dpi [75]\n";
    let (modes, list, range) = sane_options(pixma);
    assert_eq!((modes.len(), list, range), (3, vec![75, 150, 300, 600, 1200], None));
    assert_eq!(sane_mode(&modes, "Gray").as_deref(), Some("Gray"));
    // Brother's brscan names them its own way
    let brscan = "    --mode Black & White|Gray[Error Diffusion]|True Gray|24bit Color|24bit Color[Fast] [24bit Color]\n    --resolution 100|150|200|300|400|600|1200|2400|4800|9600dpi [200]\n";
    let (modes, list, _) = sane_options(brscan);
    assert_eq!(modes, ["Black & White", "Gray[Error Diffusion]", "True Gray", "24bit Color", "24bit Color[Fast]"]);
    assert_eq!(sane_mode(&modes, "Color").as_deref(), Some("24bit Color"));
    assert_eq!(sane_mode(&modes, "Gray").as_deref(), Some("True Gray"));
    assert_eq!(list[0], 100);
    let range = "    --resolution 50..2400dpi (in steps of 1) [150]\n";
    assert_eq!(sane_options(range).2, Some((50, 2400)));
    assert_eq!(sane_mode(&[], "Color"), None);
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
    let pdf = save_scans(&[(img, 150)], &out, "OCR").unwrap().remove(0);
    let text = run("pdftotext", &[&pdf, "-"]).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(text.contains("Hello printer"), "{text}");
}
