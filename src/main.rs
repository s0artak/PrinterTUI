use printertui::*;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::Line,
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
        match &mut app.mode {
            Mode::Main => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                KeyCode::Up | KeyCode::Char('k') => app.sel = app.sel.checked_sub(1).unwrap_or(ADD),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => app.sel = (app.sel + 1) % (ADD + 1),
                KeyCode::Left | KeyCode::Char('h') => app.cycle(false),
                KeyCode::Right | KeyCode::Char('l') => app.cycle(true),
                KeyCode::Char('i') | KeyCode::Char('a') if app.text().is_some() => app.mode = Mode::Insert,
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
    fn text(&mut self) -> Option<&mut String> {
        match self.sel {
            FILE => Some(&mut self.file),
            PAGES => Some(&mut self.pages),
            _ => None,
        }
    }

    fn cycle(&mut self, fwd: bool) {
        let step = |i: usize, n: usize| if n == 0 { 0 } else if fwd { (i + 1) % n } else { (i + n - 1) % n };
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
            "printer={}\ncolor={}\nduplex={}\nreverse_back={}\npaper={}\ncopies={}\nper_sheet={}\n",
            self.printers.get(self.printer).map_or("", String::as_str),
            self.color, self.duplex, self.reverse_back, PAPERS[self.paper], self.copies, PER_SHEET[self.per_sheet],
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
    let [form, status, help] = Layout::vertical([
        Constraint::Length(LABELS.len() as u16 + 2),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let lines: Vec<Line> = (0..LABELS.len())
        .map(|i| {
            let cursor = if i == app.sel && matches!(app.mode, Mode::Insert) { "_" } else { "" };
            let line = Line::from(format!(" {:<12}{}{}", LABELS[i], app.value(i), cursor));
            if i == app.sel {
                line.style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD))
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(Block::bordered().title(" PrinterTUI ")), form);

    f.render_widget(
        Paragraph::new(app.status.as_str()).wrap(Wrap { trim: false }).block(Block::bordered()),
        status,
    );
    f.render_widget(
        Line::from(match app.mode {
            Mode::Insert => " -- INSERT --   Esc/Enter done",
            _ if app.sel == PAGES => " j/k move   Enter pick pages   i type a range (1-3,7)   q quit",
            _ => " j/k move   h/l change   i edit text   Enter select   q quit",
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

fn popup(f: &mut Frame, height: u16) -> ratatui::layout::Rect {
    let a = f.area();
    let w = a.width.saturating_sub(4).min(76);
    let h = height.min(a.height);
    let area = ratatui::layout::Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h);
    f.render_widget(Clear, area);
    area
}
