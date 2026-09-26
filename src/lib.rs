//! Thin wrappers around the CUPS command line tools (lp, lpstat, lpinfo, lpadmin).

use std::process::Command;

pub const PAPERS: [&str; 5] = ["A4", "Letter", "Legal", "A5", "A3"];

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

/// Splits pages into (front, back) for manual duplex: 1st, 3rd, 5th... on the front.
pub fn split_duplex(pages: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let front = pages.iter().step_by(2).copied().collect();
    let back = pages.iter().skip(1).step_by(2).copied().collect();
    (front, back)
}

pub fn join(pages: &[u32]) -> String {
    // ponytail: no run compression ("1,2,3" not "1-3"); only matters for huge contiguous single-sided lists
    pages.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

pub struct Job<'a> {
    pub printer: &'a str,
    pub file: &'a str,
    pub color: bool,
    pub paper: &'a str,
    pub pages: Option<String>,
    pub reverse: bool,
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
    a.extend(["--".into(), job.file.into()]);
    a
}

/// Sends a job with `lp`, returns its "request id is ..." line.
pub fn submit(args: &[String]) -> Result<String, String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run("lp", &args)
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
    let (scheme, rest) = uri.split_once("://")?;
    let host = rest.split(['/', ':', '?']).next().filter(|h| !h.is_empty())?;
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
