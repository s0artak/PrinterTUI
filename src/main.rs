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
    /// Front side sent; holds the back-side job waiting for the user to flip the paper.
    Flip(Vec<String>),
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
        term.draw(|f| draw(f, app))?;
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
                    let picked = pick_file();
                    *term = ratatui::init();
                    match picked {
                        Some(p) => app.file = p,
                        None => app.status = "No file picked (install yazi, lf, ranger, nnn or fzf).".into(),
                    }
                }
                KeyCode::Enter => app.print(),
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
            Mode::Flip(back) => match k.code {
                KeyCode::Char('v') => {
                    if let Err(e) = play_tutorial() {
                        app.status = format!("Could not open the video: {e}\n\n{}", app.status);
                    }
                }
                KeyCode::Esc => {
                    app.status = "Back side cancelled.".into();
                    app.mode = Mode::Main;
                }
                KeyCode::Enter => {
                    app.status = match submit(back) {
                        Ok(id) => format!("Back side sent: {id}"),
                        Err(e) => e,
                    };
                    app.mode = Mode::Main;
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
            FILE => self.file.clone(),
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

    fn print(&mut self) {
        self.status = match self.try_print() {
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

    fn try_print(&mut self) -> Result<String, String> {
        let printer = self.printers.get(self.printer).ok_or("No printer selected")?;
        let file = expand_home(self.file.trim());
        if !std::path::Path::new(&file).is_file() {
            return Err(format!("File not found: {file}"));
        }
        let pages = self.pages.trim();
        let per_sheet = PER_SHEET[self.per_sheet];
        let job = |pages: Option<String>, reverse, collate| {
            lp_args(&Job {
                printer, file: &file, color: self.color, paper: PAPERS[self.paper], pages, reverse,
                copies: self.copies, collate, per_sheet,
            })
        };
        if !self.duplex {
            let range = (!pages.is_empty() && pages != "all").then(|| pages.to_string());
            return submit(&job(range, false, true));
        }
        let total = page_count(&file).ok_or("Double-sided needs a PDF file")?;
        // page-ranges picks pages before number-up groups them, so split whole sheet sides
        let pages = parse_ranges(pages, total)?;
        let sides: Vec<&[u32]> = pages.chunks(per_sheet as usize).collect();
        let (front, back) = split_duplex(&sides);
        let odd = front.len() > back.len();
        // Collated copies would leave a sheet without back side inside every copy when odd.
        let (front, back) = (front.concat(), back.concat());
        let front_job = job(Some(join(&front)), false, !odd);
        let back_job = job(Some(join(&back)), self.reverse_back, !odd);
        let id = submit(&front_job)?;
        if back.is_empty() {
            return Ok(format!("Only one page, nothing to flip: {id}"));
        }
        self.mode = Mode::Flip(back_job);
        Ok(format!(
            "Front side sent: {id}\n\n\
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
        ))
    }
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
        Mode::Flip(_) => {
            let area = popup(f, 14);
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
