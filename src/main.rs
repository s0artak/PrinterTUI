use printertui::*;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
    DefaultTerminal, Frame,
};
use std::cell::Cell;
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Result of a background task, applied to the app on the UI thread.
type Done = Box<dyn FnOnce(&mut App) + Send>;

const LABELS: [&str; 12] = [
    "Printer", "File", "Color", "Sides", "Back order", "Pages", "Paper", "Copies", "Per sheet", "", "", "",
];
const FILE: usize = 1;
const PAGES: usize = 5;
const PRINT: usize = 9;
const ADD: usize = 10;
const QUEUE: usize = 11;

const SCAN_LABELS: [&str; 9] = ["Scanner", "Mode", "Resolution", "Format", "Save as", "", "Page", "", ""];
const SAVE_AS: usize = 4;
const SCAN: usize = 5;
const PAGE: usize = 6;
const SAVE: usize = 7;
const DISCARD: usize = 8;

/// A scanned page: edits are always applied to the original scan.
struct ScannedPage {
    orig: String,
    /// The original, or the edited copy once `rot` / `filter` are applied.
    file: String,
    rot: u16,
    filter: usize,
    keep: bool,
    thumb: (usize, usize, Vec<u8>),
}

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
    /// Print queue popup: (job number, description), refreshed every second.
    Queue(Vec<(String, String)>, ListState),
}

struct App {
    printers: Vec<String>,
    labels: Vec<String>,
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
    scan_format: usize,
    save_as: String,
    scans: Vec<ScannedPage>,
    /// Page shown in the preview and edited by the Page row.
    cur: usize,
    /// Slow work (scanning, converting, discovery) runs in a thread so the UI keeps drawing.
    busy: Option<(String, mpsc::Receiver<Done>)>,
    /// Page rotate/filter re-render, apart from `busy` so pages can be edited while the next one scans.
    editing: Option<mpsc::Receiver<Done>>,
    /// Some(inside tmux) when the terminal speaks the kitty graphics protocol (kitty, Ghostty).
    graphics: Option<bool>,
    /// Size of the preview panel at the last draw, and the image sent to the terminal for it.
    preview_area: Cell<ratatui::layout::Rect>,
    sent: Option<(String, u16, u16)>,
    started: Instant,
    last_poll: Instant,
    sel: usize,
    status: String,
    mode: Mode,
}

fn main() -> std::io::Result<()> {
    let mut app = App {
        printers: Vec::new(),
        labels: Vec::new(),
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
        scan_format: 0,
        save_as: default_scan_name(),
        scans: Vec::new(),
        cur: 0,
        busy: None,
        editing: None,
        graphics: kitty_graphics(),
        preview_area: Cell::default(),
        sent: None,
        started: Instant::now(),
        last_poll: Instant::now(),
        sel: 0,
        status: String::new(),
        mode: Mode::Main,
    };
    app.set_printers(printers());
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
    if let (Some(tmux), Some(_)) = (app.graphics, &app.sent) {
        emit(&kitty(&format!("a=d,d=I,i={},q=2", image_id()), "", tmux));
    }
    ratatui::restore();
    stop_all();
    res
}

fn run(term: &mut DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    // first key of vim `gg` / `dd`
    let mut pending: Option<char> = None;
    loop {
        if let Some(done) = app.busy.as_ref().and_then(|(_, rx)| finished(rx)) {
            app.busy = None;
            done(app);
        }
        if let Some(done) = app.editing.as_ref().and_then(finished) {
            app.editing = None;
            done(app);
        }
        if app.last_poll.elapsed() >= Duration::from_secs(1) {
            app.last_poll = Instant::now();
            if let Mode::Flip { job, steps, .. } = &mut app.mode
                && job.as_deref().is_some_and(|id| !job_active(id))
            {
                *job = None;
                app.status = std::mem::take(steps);
            }
            if let Mode::Queue(jobs, _) = &mut app.mode {
                *jobs = queue();
            }
        }
        term.draw(|f| draw(f, app))?;
        app.sync_image();
        // short timeout keeps the spinner and the duplex animation moving
        if !event::poll(Duration::from_millis(100))? {
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
        if app.busy.is_some() && k.code == KeyCode::Enter {
            continue;
        }
        let prev = pending.take();
        let (gg, dd) = (prev == Some('g') && k.code == KeyCode::Char('g'), prev == Some('d') && k.code == KeyCode::Char('d'));
        if let KeyCode::Char(c @ ('g' | 'd')) = k.code
            && prev != Some(c)
            && !matches!(app.mode, Mode::Insert)
        {
            pending = Some(c);
        }
        match &mut app.mode {
            Mode::Main => match k.code {
                // H/L browse the scanned pages from any row
                KeyCode::Char(c @ ('H' | 'L')) if app.scan_tab && !app.scans.is_empty() => {
                    let n = app.scans.len();
                    app.cur = (app.cur + if c == 'L' { 1 } else { n - 1 }) % n;
                }
                KeyCode::Char(c) if app.scan_tab && app.sel == PAGE && !app.scans.is_empty() && app.page_key(c, dd) => {}
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
                        app.find_scanners(false);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => app.cycle(false),
                KeyCode::Right | KeyCode::Char('l') => app.cycle(true),
                KeyCode::Char('i') | KeyCode::Char('a') if app.text().is_some() => app.mode = Mode::Insert,
                KeyCode::Enter if app.scan_tab => app.scan_enter(),
                KeyCode::Enter if app.sel == ADD => app.spawn("Searching for network printers", || {
                    let found = discover();
                    Box::new(move |app: &mut App| {
                        if found.is_empty() {
                            app.status = "No network printers found.".into();
                        } else {
                            app.status = String::new();
                            app.mode = Mode::Pick(found, ListState::default().with_selected(Some(0)));
                        }
                    })
                }),
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
                KeyCode::Enter if app.sel == PAGES => app.open_pages(),
                KeyCode::Enter if app.sel == QUEUE => {
                    app.status = String::new();
                    app.mode = Mode::Queue(queue(), ListState::default().with_selected(Some(0)));
                }
                KeyCode::Enter => app.try_print(),
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
                    app.set_printers(printers());
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
            Mode::Queue(jobs, state) => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => app.mode = Mode::Main,
                KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                KeyCode::Char('g') if gg => state.select_first(),
                KeyCode::Char('G') => state.select_last(),
                KeyCode::Char('x') | KeyCode::Delete => {
                    if let Some((id, desc)) = state.selected().and_then(|i| jobs.get(i)).cloned() {
                        app.status = match cancel_job(&id) {
                            Ok(()) => format!("Cancelled {desc}"),
                            Err(e) => format!("Error: {e}"),
                        };
                        *jobs = queue();
                    }
                }
                _ => {}
            },
            Mode::Flip { job, .. } => match k.code {
                KeyCode::Esc => {
                    app.status = "Back side cancelled.".into();
                    app.mode = Mode::Main;
                }
                KeyCode::Enter if job.is_none() => {
                    let res = app.back_side();
                    app.show(res);
                }
                _ => {}
            },
        }
    }
}

/// Runs `work` in a thread; its returned closure updates the app when it finishes.
fn task(work: impl FnOnce() -> Done + Send + 'static) -> mpsc::Receiver<Done> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(work()));
    rx
}

/// A background task's result once it is done; a crashed task reports an error.
fn finished(rx: &mpsc::Receiver<Done>) -> Option<Done> {
    match rx.try_recv() {
        Ok(done) => Some(done),
        Err(mpsc::TryRecvError::Disconnected) => Some(Box::new(|app: &mut App| app.status = "Error: the background task crashed".into())),
        Err(mpsc::TryRecvError::Empty) => None,
    }
}

impl App {
    fn spawn(&mut self, msg: impl Into<String>, work: impl FnOnce() -> Done + Send + 'static) {
        self.busy = Some((msg.into(), task(work)));
    }

    fn set_printers(&mut self, printers: Vec<String>) {
        self.labels = printer_labels(&printers);
        self.printers = printers;
    }

    fn last(&self) -> usize {
        if self.scan_tab { DISCARD } else { QUEUE }
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
                3 => self.scan_format = step(self.scan_format, SCAN_FORMATS.len()),
                PAGE => self.cur = step(self.cur, self.scans.len()),
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
                3 => pick(match SCAN_FORMATS[self.scan_format] {
                    "PDF" => "PDF (all pages in one file)",
                    "OCR" => "PDF, searchable text (OCR)",
                    _ => "PNG (one file per page)",
                }),
                SAVE_AS => self.save_as.clone(),
                SCAN => "[ Scan page ]".into(),
                PAGE => match self.scans.get(self.cur) {
                    None => "none yet".into(),
                    Some(p) => {
                        let mut v = format!("< {} / {} >  [{}] keep", self.cur + 1, self.scans.len(), if p.keep { "x" } else { " " });
                        if p.rot != 0 {
                            v += &format!(" · rotated {}°", p.rot);
                        }
                        if p.filter != 0 {
                            v += &format!(" · {}", FILTERS[p.filter]);
                        }
                        v
                    }
                },
                SAVE => {
                    let (kept, all) = (self.scans.iter().filter(|p| p.keep).count(), self.scans.len());
                    let of = if kept == all { String::new() } else { format!(" of {all}") };
                    format!("[ Save {kept}{of} page{} ]", if all == 1 { "" } else { "s" })
                }
                _ => "[ Discard all ]".into(),
            };
        }
        match row {
            0 => pick(self.labels.get(self.printer).map_or("none", String::as_str)),
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
            ADD => "[ Add printer ]".into(),
            _ => "[ Print queue ]".into(),
        }
    }

    /// Opens the page selector for the first file, pre-checking the current range.
    fn open_pages(&mut self) {
        let Some(file) = split_files(&self.file).first().map(|f| expand_home(f)) else {
            return self.status = "Error: Pick a file first".into();
        };
        self.spawn("Reading pages", move || {
            let total = pdf(&file).and_then(|p| page_count(&p).ok_or("Could not read the PDF".into()));
            Box::new(move |app: &mut App| match total {
                Ok(total) => {
                    let current = parse_ranges(&app.pages, total).unwrap_or_default();
                    let on = (1..=total).map(|n| current.contains(&n)).collect();
                    app.status = String::new();
                    app.mode = Mode::Pages(on, ListState::default().with_selected(Some(0)));
                }
                Err(e) => app.status = format!("Error: {e}"),
            })
        });
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
            SCAN_MODES[self.scan_mode], SCAN_DPI[self.scan_dpi], SCAN_FORMATS[self.scan_format],
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
            "scan_format" => self.scan_format = SCAN_FORMATS.iter().position(|f| *f == v).unwrap_or(self.scan_format),
            _ => {}
        }
    }


    fn job(&self, file: &str, pages: Option<String>, reverse: bool, collate: bool) -> Vec<String> {
        lp_args(&Job {
            printer: &self.printers[self.printer], file, color: self.color, paper: PAPERS[self.paper], pages, reverse,
            copies: self.copies, collate, per_sheet: PER_SHEET[self.per_sheet],
        })
    }

    fn files(&self) -> Result<Vec<String>, String> {
        self.printers.get(self.printer).ok_or("No printer selected")?;
        let files: Vec<String> = split_files(&self.file).iter().map(|f| expand_home(f)).collect();
        if files.is_empty() {
            return Err("No file selected".into());
        }
        if let Some(f) = files.iter().find(|f| !std::path::Path::new(f).is_file()) {
            return Err(format!("File not found: {f}"));
        }
        Ok(files)
    }

    /// Converts the files to PDF in the background, then sends the jobs.
    fn try_print(&mut self) {
        let files = match self.files() {
            Ok(f) => f,
            Err(e) => return self.show(Err(e)),
        };
        self.spawn("Preparing files", move || {
            let pdfs: Result<Vec<String>, String> = files.iter().map(|f| pdf(f)).collect();
            Box::new(move |app: &mut App| {
                let res = pdfs.and_then(|p| app.print_pdfs(p));
                app.show(res)
            })
        });
    }

    fn print_pdfs(&mut self, pdfs: Vec<String>) -> Result<String, String> {
        if self.duplex {
            return self.duplex(pdfs, 0, String::new());
        }
        let pages = self.pages.trim();
        let range = (!pages.is_empty() && pages != "all").then(|| pages.to_string());
        let sent: Result<Vec<String>, String> = pdfs.iter().map(|p| submit(&self.job(p, range.clone(), false, true))).collect();
        Ok(sent?.join("\n"))
    }

    /// Prints the front of files[i..] until one needs flipping, then waits in Mode::Flip.
    fn duplex(&mut self, files: Vec<String>, mut i: usize, mut done: String) -> Result<String, String> {
        self.mode = Mode::Main;
        while let Some(file) = files.get(i) {
            i += 1;
            let head = if files.len() > 1 { format!("File {i} of {}: {}\n", files.len(), name(file)) } else { String::new() };
            let pdf = file.clone();
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
                 Enter = print back side    Esc = cancel",
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

    fn back_side(&mut self) -> Result<String, String> {
        let Mode::Flip { back, files, next, .. } = std::mem::replace(&mut self.mode, Mode::Main) else {
            return Ok(String::new());
        };
        let id = submit(&back)?;
        self.duplex(files, next, format!("Back side sent: {id}\n\n"))
    }
}

impl App {
    /// Sends the last scanned page to a kitty-protocol terminal as a virtual placement the size
    /// of the preview panel; draw_preview shows it by writing Unicode placeholder cells.
    fn sync_image(&mut self) {
        let Some(tmux) = self.graphics else { return };
        let area = self.preview_area.get();
        let want = self.preview_png().map(|p| (p, area.width, area.height));
        if want.is_some() && want != self.sent && area.width > 0 {
            let (png, c, r) = want.clone().unwrap_or_default();
            emit(&kitty(&format!("a=T,U=1,f=100,t=f,i={},q=2,c={c},r={r}", image_id()), &base64(png.as_bytes()), tmux));
            self.sent = want;
        }
    }

    fn preview_png(&self) -> Option<String> {
        self.scans.get(self.cur).map(|p| format!("{}.preview.png", p.file))
    }

    /// Page row keys; returns false for keys it does not use.
    fn page_key(&mut self, c: char, dd: bool) -> bool {
        let i = self.cur;
        match c {
            'r' => self.scans[i].rot = (self.scans[i].rot + 90) % 360,
            'R' => self.scans[i].rot = (self.scans[i].rot + 270) % 360,
            'f' => self.scans[i].filter = (self.scans[i].filter + 1) % FILTERS.len(),
            'x' | ' ' => {
                self.scans[i].keep = !self.scans[i].keep;
                return true;
            }
            '<' | '>' => {
                let j = if c == '<' { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < self.scans.len()) };
                if let Some(j) = j {
                    self.scans.swap(i, j);
                    self.cur = j;
                }
                return true;
            }
            'd' if dd => {
                self.scans.remove(i);
                self.cur = i.min(self.scans.len().saturating_sub(1));
                self.status = format!("Page {} deleted.", i + 1);
                return true;
            }
            'd' => return true, // wait for the second d
            _ => return false,
        }
        self.edit_page();
        true
    }

    /// Re-renders the current page from its original with its rotation and filter.
    fn edit_page(&mut self) {
        let p = &self.scans[self.cur];
        let (orig, rot, filter) = (p.orig.clone(), p.rot, p.filter);
        // a newer edit replaces a running one: dropping the old receiver discards its result
        self.editing = Some(task(move || {
            let res = edit_page(&orig, rot, filter);
            Box::new(move |app: &mut App| match res {
                // the page may have moved meanwhile, and only the latest edit counts
                Ok((file, thumb)) => {
                    if let Some(p) = app.scans.iter_mut().find(|p| p.orig == orig && (p.rot, p.filter) == (rot, filter)) {
                        p.file = file;
                        p.thumb = thumb;
                    }
                }
                Err(e) => app.status = format!("Error: {e}"),
            })
        }));
    }

    fn find_scanners(&mut self, full: bool) {
        self.spawn("Searching for scanners", move || {
            let list = scanners(full);
            Box::new(move |app: &mut App| {
                app.scanner = list.iter().position(|(d, _)| *d == app.scanner_pref).unwrap_or(0);
                app.status = if list.is_empty() {
                    "No scanners found. Network printers that can scan show up on their own;\nfor other scanners install SANE, then press Enter on Scanner to search again.".into()
                } else {
                    String::new()
                };
                app.scanners = Some(list);
            })
        });
    }

    fn scan_enter(&mut self) {
        match self.sel {
            0 => self.find_scanners(true),
            SCAN => self.scan_page(),
            SAVE => self.save_scans(),
            DISCARD => {
                self.scans.clear();
                self.cur = 0;
                self.status = "Scanned pages discarded.".into();
            }
            _ => {}
        }
    }

    fn scan_page(&mut self) {
        let Some((device, _)) = self.scanners.as_ref().and_then(|l| l.get(self.scanner)).cloned() else {
            return self.status = "Error: No scanner selected".into();
        };
        let dir = std::env::temp_dir().join("printertui-scan");
        let n = self.scans.len() + 1;
        // unique name: pages can be deleted and reordered, and old edits must not be reused
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let path = dir.join(format!("page-{stamp}")).to_string_lossy().into_owned();
        let (mode, dpi) = (SCAN_MODES[self.scan_mode], SCAN_DPI[self.scan_dpi]);
        self.spawn(format!("Scanning page {n}"), move || {
            let res = std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .and_then(|_| scan(&device, mode, dpi, &path))
                .and_then(|_| thumbnail(&path));
            Box::new(move |app: &mut App| {
                app.status = match res {
                    Ok(thumb) => {
                        app.scans.push(ScannedPage { orig: path.clone(), file: path, rot: 0, filter: 0, keep: true, thumb });
                        app.cur = app.scans.len() - 1;
                        format!("Scanned page {n}. Put the next page on the glass and scan again, or save.\nOn the Page row: r rotate, f filter, x keep, </> move, dd delete.\nH/L browse pages from any row.")
                    }
                    Err(e) => format!("Error: {e}"),
                }
            })
        });
    }

    fn save_scans(&mut self) {
        let files: Vec<String> = self.scans.iter().filter(|p| p.keep).map(|p| p.file.clone()).collect();
        if files.is_empty() {
            return self.status = "Error: No pages to save".into();
        }
        if self.editing.is_some() {
            return self.status = "A page edit is still running, press Enter again in a moment.".into();
        }
        let out = expand_home(self.save_as.trim());
        let out = out.trim_end_matches(".pdf").trim_end_matches(".png").to_string();
        let (format, dpi) = (SCAN_FORMATS[self.scan_format], SCAN_DPI[self.scan_dpi]);
        self.spawn("Saving", move || {
            let res = save_scans(&files, &out, format, dpi);
            Box::new(move |app: &mut App| {
                app.status = match res {
                    Ok(written) => {
                        app.scans.clear();
                        app.cur = 0;
                        app.save_as = default_scan_name();
                        let _ = app.save();
                        let written: Vec<String> = written.iter().map(|p| tilde(p)).collect();
                        format!("Saved {}", written.join(", "))
                    }
                    Err(e) => format!("Error: {e}"),
                }
            })
        });
    }
}

fn emit(s: &str) {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(s.as_bytes()).and_then(|_| stdout.flush());
}

/// Image ids are global to the terminal, so derive ours from the pid (24 bits, sent as the placeholder's RGB color).
fn image_id() -> u32 {
    std::process::id() & 0xFF_FFFF | 1
}

/// Row numbers for kitty Unicode placeholders (the first 100 of kitty's rowcolumn-diacritics.txt).
const ROW_DIACRITICS: &str = "\u{0305}\u{030D}\u{030E}\u{0310}\u{0312}\u{033D}\u{033E}\u{033F}\u{0346}\u{034A}\u{034B}\u{034C}\u{0350}\u{0351}\u{0352}\u{0357}\u{035B}\u{0363}\u{0364}\u{0365}\u{0366}\u{0367}\u{0368}\u{0369}\u{036A}\u{036B}\u{036C}\u{036D}\u{036E}\u{036F}\u{0483}\u{0484}\u{0485}\u{0486}\u{0487}\u{0592}\u{0593}\u{0594}\u{0595}\u{0597}\u{0598}\u{0599}\u{059C}\u{059D}\u{059E}\u{059F}\u{05A0}\u{05A1}\u{05A8}\u{05A9}\u{05AB}\u{05AC}\u{05AF}\u{05C4}\u{0610}\u{0611}\u{0612}\u{0613}\u{0614}\u{0615}\u{0616}\u{0617}\u{0657}\u{0658}\u{0659}\u{065A}\u{065B}\u{065D}\u{065E}\u{06D6}\u{06D7}\u{06D8}\u{06D9}\u{06DA}\u{06DB}\u{06DC}\u{06DF}\u{06E0}\u{06E1}\u{06E2}\u{06E4}\u{06E7}\u{06E8}\u{06EB}\u{06EC}\u{0730}\u{0732}\u{0733}\u{0735}\u{0736}\u{073A}\u{073D}\u{073F}\u{0740}\u{0741}\u{0743}\u{0745}\u{0747}\u{0749}\u{074A}";

/// Kitty graphics protocol support: kitty and Ghostty, directly or through tmux when
/// `allow-passthrough` is on. Returns Some(inside tmux).
fn kitty_graphics() -> Option<bool> {
    let var = |k| std::env::var(k).unwrap_or_default();
    let known = |term: &str| term.contains("kitty") || term.contains("ghostty");
    if std::env::var_os("TMUX").is_some() {
        let tmux = |args: &[&str]| std::process::Command::new("tmux").args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let outer = tmux(&["display", "-p", "#{client_termname}"]).unwrap_or_default();
        let passthrough = tmux(&["show", "-gv", "allow-passthrough"]).unwrap_or_default();
        return (known(&outer) && (passthrough == "on" || passthrough == "all")).then_some(true);
    }
    (known(&var("TERM")) || var("TERM_PROGRAM") == "ghostty" || !var("KITTY_WINDOW_ID").is_empty()).then_some(false)
}

/// `~/Documents/scan-2026-09-26_154200` (or in `~` when there is no Documents folder).
fn default_scan_name() -> String {
    let date = std::process::Command::new("date").arg("+%Y-%m-%d_%H%M%S").output();
    let date = date.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let docs = std::env::var("HOME").is_ok_and(|h| std::path::Path::new(&h).join("Documents").is_dir());
    format!("~/{}scan-{date}", if docs { "Documents/" } else { "" })
}

/// The file itself if it is a PDF, otherwise a PDF converted by LibreOffice.
fn pdf(file: &str) -> Result<String, String> {
    if page_count(file).is_some() { Ok(file.into()) } else { to_pdf(file) }
}

fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Inverse of expand_home, for showing paths.
fn tilde(p: &str) -> String {
    match std::env::var("HOME").ok().and_then(|h| p.strip_prefix(&h).map(str::to_string)) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
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

    let tick = app.started.elapsed().as_millis() as usize;
    let text = match &app.busy {
        Some((msg, _)) => format!("{msg}... {}", ['|', '/', '-', '\\'][tick / 150 % 4]),
        None if app.editing.is_some() => format!("Editing page... {}", ['|', '/', '-', '\\'][tick / 150 % 4]),
        // the duplex popup already shows the status
        None if matches!(app.mode, Mode::Flip { .. }) => String::new(),
        None => app.status.clone(),
    };
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).block(Block::bordered()), status);
    f.render_widget(
        Line::from(match app.mode {
            Mode::Insert => " -- INSERT --   Esc/Enter done",
            _ if app.sel == PAGE && app.scan_tab => " H/L page   </> move   r/R rotate   f filter   x keep   dd delete",
            _ if app.scan_tab && !app.scans.is_empty() => " j/k move   h/l change   H/L page   Enter select   Tab print/scan   q quit",
            _ if app.sel == PAGES && !app.scan_tab => " j/k move   Enter pick pages   i type a range (1-3,7)   q quit",
            _ => " j/k move   h/l change   i edit text   Enter select   Tab print/scan   q quit",
        })
            .style(Style::new().add_modifier(Modifier::DIM)),
        help,
    );

    match &app.mode {
        Mode::Main | Mode::Insert => {}
        Mode::Pick(found, state) => {
            let area = popup(f, 76, found.len() as u16 + 2);
            let items: Vec<ListItem> = found.iter().map(|(n, u)| ListItem::new(format!("{n}  {u}"))).collect();
            let list = List::new(items)
                .block(Block::bordered().title(" Add printer (Enter add, Esc back) "))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, &mut state.clone());
        }
        Mode::Pages(on, state) => {
            let area = popup(f, 76, on.len() as u16 + 2);
            let items: Vec<ListItem> =
                on.iter().enumerate().map(|(i, b)| ListItem::new(format!(" [{}] Page {}", if *b { "x" } else { " " }, i + 1))).collect();
            let n = on.iter().filter(|b| **b).count();
            let list = List::new(items)
                .block(Block::bordered().title(format!(" Pages {n}/{} (Space toggle, a all, Enter ok, Esc back) ", on.len())))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, &mut state.clone());
        }
        Mode::Queue(jobs, state) => {
            let area = popup(f, 90, jobs.len().max(1) as u16 + 2);
            let items: Vec<ListItem> = if jobs.is_empty() {
                vec![ListItem::new(" No pending jobs")]
            } else {
                jobs.iter().map(|(id, desc)| ListItem::new(format!(" #{id}  {desc}"))).collect()
            };
            let list = List::new(items)
                .block(Block::bordered().title(" Print queue (x cancel job, Esc back) ").title_bottom(format!(" {} ", app.status)))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            // the list shrinks as jobs finish, keep the cursor on a row
            let mut state = *state;
            if state.selected().is_some_and(|i| i >= jobs.len()) {
                state.select(jobs.len().checked_sub(1));
            }
            f.render_stateful_widget(list, area, &mut state);
        }
        Mode::Flip { job, .. } => {
            let area = popup(f, 100, 18);
            let block = Block::bordered().title(" Manual duplex ");
            let [text, anim] = Layout::horizontal([Constraint::Min(30), Constraint::Length(31)]).areas(block.inner(area));
            f.render_widget(block, area);
            f.render_widget(Paragraph::new(app.status.as_str()).wrap(Wrap { trim: false }), text);
            if job.is_none() {
                let lines: Vec<Line> = duplex_frame(tick / 700).into_iter().map(Line::from).collect();
                f.render_widget(Paragraph::new(lines), anim);
            }
        }
    }
}

/// One frame of the flip animation (front view, you standing in front of the printer):
/// the printed sheet turns top over bottom and goes into the rear tray.
fn duplex_frame(tick: usize) -> Vec<String> {
    const FRONT: [&str; 5] = ["TOP", "", "1", "", ""];
    const BACK: [&str; 5] = ["", "", "blank", "", "top ▼"];
    // (card top row, card height, printed side visible, caption)
    const STEPS: [(usize, usize, bool, &str); 14] = [
        (1, 7, true, "Printed side facing you,\ntext upright"),
        (1, 7, true, "Printed side facing you,\ntext upright"),
        (2, 5, true, "Flip it top over bottom"),
        (3, 3, true, "Flip it top over bottom"),
        (4, 1, true, "Flip it top over bottom"),
        (3, 3, false, "Flip it top over bottom"),
        (2, 5, false, "Flip it top over bottom"),
        (1, 7, false, "Blank side facing you,\ntop edge at the bottom"),
        (1, 7, false, "Blank side facing you,\ntop edge at the bottom"),
        (3, 7, false, "Put it in the rear tray,\ntop edge going in first"),
        (5, 7, false, "Put it in the rear tray,\ntop edge going in first"),
        (7, 7, false, "Put it in the rear tray,\ntop edge going in first"),
        (9, 7, false, "Press Enter"),
        (9, 7, false, "Press Enter"),
    ];
    let (y, h, front, caption) = STEPS[tick % STEPS.len()];
    let content = if front { FRONT } else { BACK };
    let pad = " ".repeat(9);
    let mut rows = vec![String::new(); 9];
    for r in y..(y + h).min(9) {
        let line = match r - y {
            _ if h == 1 => "─────────────".to_string(),
            0 => "┌───────────┐".to_string(),
            i if i == h - 1 => "└───────────┘".to_string(),
            // squashed card: sample the lines evenly
            i => format!("│{:^11}│", content[(2 * (i - 1) + 1) * 5 / (2 * (h - 2))]),
        };
        rows[r] = format!("{pad}{line}");
    }
    rows.extend(["  ┌─────────────────────────┐".into(), "  │         PRINTER         │".into(), "  └─────────────────────────┘".into(), String::new()]);
    rows.extend(caption.lines().map(|c| format!("{c:^31}")));
    rows
}

/// Grayscale half-block rendering of the last scanned page, two pixels per cell.
fn draw_preview(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let block = Block::bordered().title(match app.scans.get(app.cur) {
        None => " Preview ".to_string(),
        Some(p) => format!(" Preview: page {} of {}{} ", app.cur + 1, app.scans.len(), if p.keep { "" } else { " (not saved)" }),
    });
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.preview_area.set(inner);
    if app.graphics.is_some() {
        // placeholder cells: U+10EEEE, the row as a diacritic on the first cell (the terminal infers
        // the columns), and the image id as the foreground color
        let want = app.preview_png().map(|p| (p, inner.width, inner.height));
        if want.is_some() && want == app.sent {
            let id = image_id();
            let color = Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8);
            for (row, diacritic) in (0..inner.height).zip(ROW_DIACRITICS.chars()) {
                for col in 0..inner.width {
                    let symbol = if col == 0 { format!("\u{10EEEE}{diacritic}") } else { "\u{10EEEE}".into() };
                    f.buffer_mut()[(inner.x + col, inner.y + row)].set_symbol(&symbol).set_fg(color);
                }
            }
        }
        return;
    }
    let Some((w, h, px)) = app.scans.get(app.cur).map(|p| &p.thumb) else { return };
    let (cols, rows) = (inner.width as usize, inner.height as usize * 2);
    let scale = (cols as f32 / *w as f32).min(rows as f32 / *h as f32);
    let (ow, oh) = (((*w as f32 * scale) as usize).max(1), ((*h as f32 * scale) as usize).max(2) & !1);
    let small = downscale(*w, *h, px, ow, oh);
    // contrast stretch (30%..97%) so text blocks stand out from the paper
    let gray = |v: u8| {
        let v = ((v as i32 - 77) * 255 / 170).clamp(0, 255) as u8;
        Color::Rgb(v, v, v)
    };
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

fn popup(f: &mut Frame, width: u16, height: u16) -> ratatui::layout::Rect {
    let a = f.area();
    let w = a.width.saturating_sub(4).min(width);
    let h = height.min(a.height);
    let area = ratatui::layout::Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h);
    f.render_widget(Clear, area);
    area
}
