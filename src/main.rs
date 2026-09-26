use printertui::*;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
    DefaultTerminal, Frame,
};

const LABELS: [&str; 11] = [
    "Printer", "File", "Color", "Sides", "Back order", "Pages", "Paper", "Copies", "Per sheet", "", "",
];
const FILE: usize = 1;
const PAGES: usize = 5;
const PRINT: usize = 9;
const ADD: usize = 10;

const SCAN_LABELS: [&str; 8] = ["Scanner", "Mode", "Resolution", "Format", "Save as", "", "", ""];
const SAVE_AS: usize = 4;
const SCAN: usize = 5;
const SAVE: usize = 6;
const DISCARD: usize = 7;

enum Mode {
    Main,
    /// Editing the selected text field (vim-style, entered with `i`).
    Insert,
    Pick(Vec<(String, String)>, ListState),
    /// Page selector: one checkbox per page of the first file.
    Pages(Vec<bool>, ListState),
    /// Manual duplex: `job` is the front job while it is still printing, `steps` the flip
    /// instructions shown after it, `back` the back-side job, `files[next..]` the files still to print.
    Flip { job: Option<String>, steps: String, back: Vec<String>, files: Vec<String>, next: usize },
}

struct App {
    printers: Vec<String>,
    printer: usize,
    file: String,
    color: bool,
    duplex: bool,
    reverse_back: bool,
    pages: String,
    paper: usize,
    copies: u32,
    per_sheet: usize,
    scan_tab: bool,
    /// None until the Scan tab is first opened (discovery takes a few seconds).
    scanners: Option<Vec<(String, String)>>,
    scanner: usize,
    scanner_pref: String,
    scan_mode: usize,
    scan_dpi: usize,
    scan_pdf: bool,
    save_as: String,
    scans: Vec<String>,
    thumb: Option<(usize, usize, Vec<u8>)>,
    sel: usize,
    status: String,
    mode: Mode,
}

fn main() -> std::io::Result<()> {
    let mut app = App {
        printers: printers(),
        printer: 0,
        file: std::env::args().nth(1).unwrap_or_default(),
        color: false,
        duplex: false,
        reverse_back: false,
        pages: String::new(),
        paper: 0,
        copies: 1,
        per_sheet: 0,
        scan_tab: false,
        scanners: None,
        scanner: 0,
        scanner_pref: String::new(),
        scan_mode: 0,
        scan_dpi: 1,
        scan_pdf: true,
        save_as: default_scan_name(),
        scans: Vec::new(),
        thumb: None,
        sel: 0,
        status: String::new(),
        mode: Mode::Main,
    };
    if let Some(text) = config_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        for (k, v) in parse_config(&text) {
            app.apply(k, v);
        }
    }
    if app.printers.is_empty() {
        app.status = "No printers configured. Select [ Add printer ].".into();
    }
    let mut term = ratatui::init();
    let res = run(&mut term, &mut app);
    ratatui::restore();
    res
}

fn run(term: &mut DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    let mut pending_g = false;
    loop {
        if let Mode::Flip { job, steps, .. } = &mut app.mode
            && job.as_deref().is_some_and(|id| !job_active(id))
        {
            *job = None;
            app.status = std::mem::take(steps);
        }
        term.draw(|f| draw(f, app))?;
        if !event::poll(std::time::Duration::from_millis(500))? {
            continue;
        }
        let Event::Key(k) = event::read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(());
        }
        // vim `gg`: a second `g` right after the first
        let gg = pending_g && k.code == KeyCode::Char('g');
        pending_g = !gg && k.code == KeyCode::Char('g') && !matches!(app.mode, Mode::Insert);
        match &mut app.mode {
            Mode::Main => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => app.sel = app.sel.checked_sub(1).unwrap_or(app.last()),
                KeyCode::Down | KeyCode::Char('j') => app.sel = (app.sel + 1) % (app.last() + 1),
                KeyCode::Char('g') if gg => app.sel = 0,
                KeyCode::Char('G') => app.sel = app.last(),
                KeyCode::Tab | KeyCode::BackTab => {
                    app.scan_tab = !app.scan_tab;
                    app.sel = 0;
                    app.status = String::new();
                    if app.scan_tab && app.scanners.is_none() {
                        app.find_scanners(term);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => app.cycle(false),
                KeyCode::Right | KeyCode::Char('l') => app.cycle(true),
                KeyCode::Char('i') | KeyCode::Char('a') if app.text().is_some() => app.mode = Mode::Insert,
                KeyCode::Enter if app.scan_tab => app.scan_enter(term),
                KeyCode::Enter if app.sel == ADD => {
                    app.status = "Searching for network printers...".into();
                    term.draw(|f| draw(f, app))?;
                    let found = discover();
                    if found.is_empty() {
                        app.status = "No network printers found.".into();
                    } else {
                        app.mode = Mode::Pick(found, ListState::default().with_selected(Some(0)));
                    }
                }
                KeyCode::Enter if app.sel == FILE => {
                    ratatui::restore();
                    let picked = pick_files();
                    *term = ratatui::init();
                    if picked.is_empty() {
                        app.status = "No file picked (install yazi, lf, ranger, nnn or fzf).".into();
                    } else {
                        app.file = picked.join("; ");
                    }
                }
                KeyCode::Enter if app.sel == PAGES => {
                    let res = app.open_pages(term);
                    if let Err(e) = res {
                        app.status = format!("Error: {e}");
                    }
                }
                KeyCode::Enter => {
                    let res = app.try_print(term);
                    app.show(res);
                }
                _ => {}
            },
            Mode::Insert => match k.code {
                KeyCode::Esc | KeyCode::Enter => app.mode = Mode::Main,
                KeyCode::Backspace => {
                    app.text().map(String::pop);
                }
                KeyCode::Char(c) => {
                    if let Some(t) = app.text() {
                        t.push(c)
                    }
                }
                _ => {}
            },
            Mode::Pick(found, state) => match k.code {
                KeyCode::Esc => app.mode = Mode::Main,
                KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                KeyCode::Char('g') if gg => state.select_first(),
                KeyCode::Char('G') => state.select_last(),
                KeyCode::Enter => {
                    let (name, uri) = found[state.selected().unwrap_or(0)].clone();
                    ratatui::restore();
                    println!("Adding {name} ({uri}), sudo may ask for your password...");
                    let res = add_printer(&name, &uri);
                    *term = ratatui::init();
                    app.status = match res {
                        Ok(()) => format!("Added printer {name}."),
                        Err(e) => e,
                    };
                    app.printers = printers();
                    app.printer = app.printers.iter().position(|p| *p == name).unwrap_or(0);
                    app.mode = Mode::Main;
                }
                _ => {}
            },
            Mode::Pages(on, state) => {
                let i = state.selected().unwrap_or(0);
                match k.code {
                    KeyCode::Esc => app.mode = Mode::Main,
                    KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                    KeyCode::Char('g') if gg => state.select_first(),
                    KeyCode::Char('G') => state.select_last(),
                    KeyCode::Char(' ') | KeyCode::Char('x') => on[i] = !on[i],
                    KeyCode::Char('a') => {
                        let all = on.iter().all(|b| *b);
                        on.iter_mut().for_each(|b| *b = !all);
                    }
                    KeyCode::Enter => {
                        let picked: Vec<u32> = (1..).zip(on.iter()).filter(|(_, b)| **b).map(|(n, _)| n).collect();
                        if picked.is_empty() {
                            app.status = "Select at least one page.".into();
                        } else {
                            app.pages = if picked.len() == on.len() { String::new() } else { join(&picked) };
                            app.mode = Mode::Main;
                        }
                    }
                    _ => {}
                }
            }
            Mode::Flip { job, .. } => match k.code {
                KeyCode::Char('v') => {
                    if let Err(e) = play_tutorial() {
                        app.status = format!("Could not open the video: {e}\n\n{}", app.status);
                    }
                }
                KeyCode::Esc => {
                    app.status = "Back side cancelled.".into();
                    app.mode = Mode::Main;
                }
                KeyCode::Enter if job.is_none() => {
                    let res = app.back_side(term);
                    app.show(res);
                }
                _ => {}
            },
        }
    }
}

impl App {
    fn last(&self) -> usize {
        if self.scan_tab { DISCARD } else { ADD }
    }

    fn text(&mut self) -> Option<&mut String> {
        if self.scan_tab {
            return (self.sel == SAVE_AS).then_some(&mut self.save_as);
        }
        match self.sel {
            FILE => Some(&mut self.file),
            PAGES => Some(&mut self.pages),
            _ => None,
        }
    }

    fn cycle(&mut self, fwd: bool) {
        let step = |i: usize, n: usize| if n == 0 { 0 } else if fwd { (i + 1) % n } else { (i + n - 1) % n };
        if self.scan_tab {
            match self.sel {
                0 => self.scanner = step(self.scanner, self.scanners.as_ref().map_or(0, Vec::len)),
                1 => self.scan_mode = step(self.scan_mode, SCAN_MODES.len()),
                2 => self.scan_dpi = step(self.scan_dpi, SCAN_DPI.len()),
                3 => self.scan_pdf = !self.scan_pdf,
                _ => {}
            }
            return;
        }
        match self.sel {
            0 => self.printer = step(self.printer, self.printers.len()),
            2 => self.color = !self.color,
            3 => self.duplex = !self.duplex,
            4 => self.reverse_back = !self.reverse_back,
            6 => self.paper = step(self.paper, PAPERS.len()),
            7 => self.copies = if fwd { self.copies + 1 } else { (self.copies - 1).max(1) },
            8 => self.per_sheet = step(self.per_sheet, PER_SHEET.len()),
            _ => {}
        }
    }

    fn value(&self, row: usize) -> String {
        let pick = |s: &str| format!("< {s} >");
        if self.scan_tab {
            return match row {
                0 => pick(self.scanners.as_ref().and_then(|l| l.get(self.scanner)).map_or("none", |(_, d)| d.as_str())),
                1 => pick(SCAN_MODES[self.scan_mode]),
                2 => pick(&format!("{} dpi", SCAN_DPI[self.scan_dpi])),
                3 => pick(if self.scan_pdf { "PDF (all pages in one file)" } else { "PNG (one file per page)" }),
                SAVE_AS => self.save_as.clone(),
                SCAN => "[ Scan page ]".into(),
                SAVE => format!("[ Save {} page{} ]", self.scans.len(), if self.scans.len() == 1 { "" } else { "s" }),
                _ => "[ Discard ]".into(),
            };
        }
        match row {
            0 => pick(self.printers.get(self.printer).map_or("none", String::as_str)),
            FILE if matches!(self.mode, Mode::Insert) => self.file.clone(),
            FILE => match split_files(&self.file)[..] {
                [] => String::new(),
                [ref one] => one.clone(),
                ref many => {
                    let names: Vec<&str> = many.iter().map(|f| name(f)).collect();
                    format!("{} files: {}", many.len(), names.join(", "))
                }
            },
            2 => pick(if self.color { "Color" } else { "Grayscale" }),
            3 => pick(if self.duplex { "Double-sided (manual)" } else { "Single-sided" }),
            4 => pick(if self.reverse_back { "Reversed" } else { "Normal" }),
            PAGES if self.pages.is_empty() && !matches!(self.mode, Mode::Insert) => "all".into(),
            PAGES => self.pages.clone(),
            6 => pick(PAPERS[self.paper]),
            7 => pick(&self.copies.to_string()),
            8 => pick(&PER_SHEET[self.per_sheet].to_string()),
            PRINT => "[ Print ]".into(),
            _ => "[ Add printer ]".into(),
        }
    }

    /// Opens the page selector for the first file, pre-checking the current range.
    fn open_pages(&mut self, term: &mut DefaultTerminal) -> Result<(), String> {
        let file = split_files(&self.file).first().map(|f| expand_home(f)).ok_or("Pick a file first")?;
        let pdf = self.pdf(term, &file)?;
        let total = page_count(&pdf).ok_or("Could not read the PDF")?;
        let current = parse_ranges(&self.pages, total).unwrap_or_default();
        let on = (1..=total).map(|n| current.contains(&n)).collect();
        self.status = String::new();
        self.mode = Mode::Pages(on, ListState::default().with_selected(Some(0)));
        Ok(())
    }

    fn show(&mut self, res: Result<String, String>) {
        self.status = match res {
            Ok(s) => match self.save() {
                Ok(()) => s,
                Err(e) => format!("{s}\n\nCould not save settings: {e}"),
            },
            Err(e) => format!("Error: {e}"),
        }
    }

    /// Remembers the settings of the last successful print (not the file or page range).
    fn save(&self) -> std::io::Result<()> {
        let path = config_path().ok_or(std::io::Error::other("HOME is not set"))?;
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
        std::fs::write(path, format!(
            "printer={}\ncolor={}\nduplex={}\nreverse_back={}\npaper={}\ncopies={}\nper_sheet={}\n\
             scanner={}\nscan_mode={}\nscan_dpi={}\nscan_format={}\n",
            self.printers.get(self.printer).map_or("", String::as_str),
            self.color, self.duplex, self.reverse_back, PAPERS[self.paper], self.copies, PER_SHEET[self.per_sheet],
            self.scanners.as_ref().and_then(|l| l.get(self.scanner)).map_or(self.scanner_pref.as_str(), |(d, _)| d.as_str()),
            SCAN_MODES[self.scan_mode], SCAN_DPI[self.scan_dpi], if self.scan_pdf { "PDF" } else { "PNG" },
        ))
    }

    /// Applies one saved setting; unknown keys and invalid values are ignored.
    fn apply(&mut self, key: &str, v: &str) {
        let b = v.parse().ok();
        match key {
            "printer" => self.printer = self.printers.iter().position(|p| p == v).unwrap_or(self.printer),
            "color" => self.color = b.unwrap_or(self.color),
            "duplex" => self.duplex = b.unwrap_or(self.duplex),
            "reverse_back" => self.reverse_back = b.unwrap_or(self.reverse_back),
            "paper" => self.paper = PAPERS.iter().position(|p| *p == v).unwrap_or(self.paper),
            "copies" => self.copies = v.parse().ok().filter(|n| *n > 0).unwrap_or(self.copies),
            "per_sheet" => self.per_sheet = PER_SHEET.iter().position(|n| n.to_string() == v).unwrap_or(self.per_sheet),
            "scanner" => self.scanner_pref = v.to_string(),
            "scan_mode" => self.scan_mode = SCAN_MODES.iter().position(|m| *m == v).unwrap_or(self.scan_mode),
            "scan_dpi" => self.scan_dpi = SCAN_DPI.iter().position(|d| d.to_string() == v).unwrap_or(self.scan_dpi),
            "scan_format" => self.scan_pdf = v != "PNG",
            _ => {}
        }
    }


    fn job(&self, file: &str, pages: Option<String>, reverse: bool, collate: bool) -> Vec<String> {
        lp_args(&Job {
            printer: &self.printers[self.printer], file, color: self.color, paper: PAPERS[self.paper], pages, reverse,
            copies: self.copies, collate, per_sheet: PER_SHEET[self.per_sheet],
        })
    }

    /// The file itself if it is a PDF, otherwise a PDF converted by LibreOffice.
    fn pdf(&mut self, term: &mut DefaultTerminal, file: &str) -> Result<String, String> {
        if page_count(file).is_some() {
            return Ok(file.into());
        }
        self.status = format!("Converting {} to PDF...", name(file));
        let _ = term.draw(|f| draw(f, self));
        to_pdf(file)
    }

    fn try_print(&mut self, term: &mut DefaultTerminal) -> Result<String, String> {
        self.printers.get(self.printer).ok_or("No printer selected")?;
        let files: Vec<String> = split_files(&self.file).iter().map(|f| expand_home(f)).collect();
        if files.is_empty() {
            return Err("No file selected".into());
        }
        if let Some(f) = files.iter().find(|f| !std::path::Path::new(f).is_file()) {
            return Err(format!("File not found: {f}"));
        }
        if self.duplex {
            return self.duplex(term, files, 0, String::new());
        }
        let pages = self.pages.trim();
        let range = (!pages.is_empty() && pages != "all").then(|| pages.to_string());
        let mut sent = Vec::new();
        for f in &files {
            let pdf = self.pdf(term, f)?;
            sent.push(submit(&self.job(&pdf, range.clone(), false, true))?);
        }
        Ok(sent.join("\n"))
    }

    /// Prints the front of files[i..] until one needs flipping, then waits in Mode::Flip.
    fn duplex(&mut self, term: &mut DefaultTerminal, files: Vec<String>, mut i: usize, mut done: String) -> Result<String, String> {
        self.mode = Mode::Main;
        while let Some(file) = files.get(i) {
            i += 1;
            let head = if files.len() > 1 { format!("File {i} of {}: {}\n", files.len(), name(file)) } else { String::new() };
            let pdf = self.pdf(term, file)?;
            let total = page_count(&pdf).ok_or("Could not read the PDF")?;
            // page-ranges picks pages before number-up groups them, so split whole sheet sides
            let pages = parse_ranges(&self.pages, total)?;
            let sides: Vec<&[u32]> = pages.chunks(PER_SHEET[self.per_sheet] as usize).collect();
            let (front, back) = split_duplex(&sides);
            // Collated copies would leave a sheet without back side inside every copy when odd.
            let odd = front.len() > back.len();
            let (front, back) = (front.concat(), back.concat());
            let id = submit(&self.job(&pdf, Some(join(&front)), false, !odd))?;
            if back.is_empty() {
                done += &format!("{head}Only one page, nothing to flip: {id}\n\n");
                continue;
            }
            let steps = format!(
                "{done}{head}Front side sent: {id}\n\n\
                 1. Wait until the printer stops.\n\
                 2. Take the whole stack out. Do not change the order.{}\n\
                 3. Flip it: printed side facing the BACK of the printer,\n   top of the page going in first (pointing down).\n\
                 4. Put it in the input tray.\n\n\
                 Enter = print back side    v = watch how (video)    Esc = cancel",
                match (odd, self.copies) {
                    (false, _) => String::new(),
                    (true, 1) => "\n   Put the top sheet aside, it has no back side.".into(),
                    (true, n) => format!("\n   Put the top {n} sheets aside (no back side, copies are uncollated)."),
                }
            );
            let wait = format!("{done}{head}Front side sent: {id}\n\nPrinting front side, please wait...\n\nEsc = cancel");
            // unparsable lp output: "" is never listed as active, so the steps show right away
            let job = Some(job_id(&id).unwrap_or_default().to_string());
            let back = self.job(&pdf, Some(join(&back)), self.reverse_back, !odd);
            self.mode = Mode::Flip { job, steps, back, files, next: i };
            return Ok(wait);
        }
        Ok(done.trim_end().to_string())
    }

    fn back_side(&mut self, term: &mut DefaultTerminal) -> Result<String, String> {
        let Mode::Flip { back, files, next, .. } = std::mem::replace(&mut self.mode, Mode::Main) else {
            return Ok(String::new());
        };
        let id = submit(&back)?;
        self.duplex(term, files, next, format!("Back side sent: {id}\n\n"))
    }
}

impl App {
    fn find_scanners(&mut self, term: &mut DefaultTerminal) {
        self.status = "Searching for scanners...".into();
        let _ = term.draw(|f| draw(f, self));
        let list = scanners();
        self.scanner = list.iter().position(|(d, _)| *d == self.scanner_pref).unwrap_or(0);
        self.status = if list.is_empty() {
            "No scanners found. Install sane and sane-airscan (network scanners),\nthen press Enter on Scanner to search again.".into()
        } else {
            String::new()
        };
        self.scanners = Some(list);
    }

    fn scan_enter(&mut self, term: &mut DefaultTerminal) {
        let res = match self.sel {
            0 => return self.find_scanners(term),
            SCAN => self.scan_page(term),
            SAVE => self.save_scans(),
            DISCARD => {
                self.scans.clear();
                self.thumb = None;
                Ok("Scanned pages discarded.".into())
            }
            _ => return,
        };
        self.status = res.unwrap_or_else(|e| format!("Error: {e}"));
    }

    fn scan_page(&mut self, term: &mut DefaultTerminal) -> Result<String, String> {
        let (device, _) = self.scanners.as_ref().and_then(|l| l.get(self.scanner)).ok_or("No scanner selected")?.clone();
        let dir = std::env::temp_dir().join("printertui-scan");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let n = self.scans.len() + 1;
        let path = dir.join(format!("page-{n}.png")).to_string_lossy().into_owned();
        self.status = format!("Scanning page {n}...");
        let _ = term.draw(|f| draw(f, self));
        // ponytail: blocks the UI while the scanner works; move to a thread if cancelling mid-scan matters
        scan(&device, SCAN_MODES[self.scan_mode], SCAN_DPI[self.scan_dpi], &path)?;
        self.thumb = thumbnail(&path).ok();
        self.scans.push(path);
        Ok(format!("Scanned page {n}. Put the next page on the glass and scan again, or save."))
    }

    fn save_scans(&mut self) -> Result<String, String> {
        if self.scans.is_empty() {
            return Err("Nothing scanned yet".into());
        }
        let out = expand_home(self.save_as.trim());
        let out = out.trim_end_matches(".pdf").trim_end_matches(".png");
        let written = save_scans(&self.scans, out, self.scan_pdf, SCAN_MODES[self.scan_mode] == "Lineart", SCAN_DPI[self.scan_dpi])?;
        self.scans.clear();
        self.thumb = None;
        self.save_as = default_scan_name();
        let _ = self.save();
        Ok(format!("Saved {}", written.join(", ")))
    }
}

/// `~/Documents/scan-2026-09-26_154200` (or in `~` when there is no Documents folder).
fn default_scan_name() -> String {
    let date = std::process::Command::new("date").arg("+%Y-%m-%d_%H%M%S").output();
    let date = date.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let docs = std::env::var("HOME").is_ok_and(|h| std::path::Path::new(&h).join("Documents").is_dir());
    format!("~/{}scan-{date}", if docs { "Documents/" } else { "" })
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Writes the embedded duplex tutorial to a temp file and opens it with the default video player.
fn play_tutorial() -> std::io::Result<()> {
    let path = std::env::temp_dir().join("printertui-duplex.mp4");
    std::fs::write(&path, include_bytes!("../assets/duplex.mp4"))?;
    std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
}

fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_string(),
    }
}

fn draw(f: &mut Frame, app: &App) {
    let labels: &[&str] = if app.scan_tab { &SCAN_LABELS } else { &LABELS };
    let [main, help] = Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(f.area());
    let (left, preview) = if app.scan_tab {
        let [l, r] = Layout::horizontal([Constraint::Min(40), Constraint::Percentage(45)]).areas(main);
        (l, Some(r))
    } else {
        (main, None)
    };
    let [form, status] =
        Layout::vertical([Constraint::Length(labels.len() as u16 + 2), Constraint::Min(3)]).areas(left);

    let lines: Vec<Line> = (0..labels.len())
        .map(|i| {
            let cursor = if i == app.sel && matches!(app.mode, Mode::Insert) { "_" } else { "" };
            let line = Line::from(format!(" {:<12}{}{}", labels[i], app.value(i), cursor));
            if i == app.sel {
                line.style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD))
            } else {
                line
            }
        })
        .collect();
    let tab = |name: &'static str, on: bool| {
        Span::styled(format!(" {name} "), if on { Style::new().add_modifier(Modifier::REVERSED) } else { Style::new() })
    };
    let title = Line::from(vec![Span::raw(" PrinterTUI  "), tab("Print", !app.scan_tab), tab("Scan", app.scan_tab), Span::raw(" ")]);
    f.render_widget(Paragraph::new(lines).block(Block::bordered().title(title)), form);
    if let Some(area) = preview {
        draw_preview(f, app, area);
    }

    f.render_widget(
        Paragraph::new(app.status.as_str()).wrap(Wrap { trim: false }).block(Block::bordered()),
        status,
    );
    f.render_widget(
        Line::from(match app.mode {
            Mode::Insert => " -- INSERT --   Esc/Enter done",
            _ if app.sel == PAGES && !app.scan_tab => " j/k move   Enter pick pages   i type a range (1-3,7)   q quit",
            _ => " j/k move   h/l change   i edit text   Enter select   Tab print/scan   q quit",
        })
            .style(Style::new().add_modifier(Modifier::DIM)),
        help,
    );

    match &app.mode {
        Mode::Main | Mode::Insert => {}
        Mode::Pick(found, state) => {
            let area = popup(f, found.len() as u16 + 2);
            let items: Vec<ListItem> = found.iter().map(|(n, u)| ListItem::new(format!("{n}  {u}"))).collect();
            let list = List::new(items)
                .block(Block::bordered().title(" Add printer (Enter add, Esc back) "))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, &mut state.clone());
        }
        Mode::Pages(on, state) => {
            let area = popup(f, on.len() as u16 + 2);
            let items: Vec<ListItem> =
                on.iter().enumerate().map(|(i, b)| ListItem::new(format!(" [{}] Page {}", if *b { "x" } else { " " }, i + 1))).collect();
            let n = on.iter().filter(|b| **b).count();
            let list = List::new(items)
                .block(Block::bordered().title(format!(" Pages {n}/{} (Space toggle, a all, Enter ok, Esc back) ", on.len())))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, &mut state.clone());
        }
        Mode::Flip { .. } => {
            let area = popup(f, 18);
            f.render_widget(
                Paragraph::new(app.status.as_str())
                    .wrap(Wrap { trim: false })
                    .block(Block::bordered().title(" Manual duplex ")),
                area,
            );
        }
    }
}

/// Grayscale half-block rendering of the last scanned page, two pixels per cell.
fn draw_preview(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let block = Block::bordered().title(match app.scans.len() {
        0 => " Preview ".to_string(),
        n => format!(" Preview: page {n} "),
    });
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some((w, h, px)) = &app.thumb else { return };
    let (cols, rows) = (inner.width as usize, inner.height as usize * 2);
    let scale = (cols as f32 / *w as f32).min(rows as f32 / *h as f32);
    let (ow, oh) = (((*w as f32 * scale) as usize).max(1), ((*h as f32 * scale) as usize).max(2) & !1);
    let small = downscale(*w, *h, px, ow, oh);
    let gray = |v: u8| Color::Rgb(v, v, v);
    let lines: Vec<Line> = (0..oh / 2)
        .map(|y| {
            let spans: Vec<Span> = (0..ow)
                .map(|x| Span::styled("▀", Style::new().fg(gray(small[2 * y * ow + x])).bg(gray(small[(2 * y + 1) * ow + x]))))
                .collect();
            Line::from(spans).centered()
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn popup(f: &mut Frame, height: u16) -> ratatui::layout::Rect {
    let a = f.area();
    let w = a.width.saturating_sub(4).min(76);
    let h = height.min(a.height);
    let area = ratatui::layout::Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h);
    f.render_widget(Clear, area);
    area
}
