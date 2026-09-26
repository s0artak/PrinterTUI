use printertui::*;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
    DefaultTerminal, Frame,
};

const LABELS: [&str; 9] = [
    "Printer", "File", "Color", "Sides", "Back order", "Pages", "Paper", "", "",
];
const FILE: usize = 1;
const PAGES: usize = 5;
const PRINT: usize = 7;
const ADD: usize = 8;

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
        sel: 0,
        status: String::new(),
        mode: Mode::Main,
    };
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
            PRINT => "[ Print ]".into(),
            _ => "[ Add printer ]".into(),
        }
    }

    fn print(&mut self) {
        self.status = match self.try_print() {
            Ok(s) => s,
            Err(e) => format!("Error: {e}"),
        }
    }

    fn try_print(&mut self) -> Result<String, String> {
        let printer = self.printers.get(self.printer).ok_or("No printer selected")?;
        let file = expand_home(self.file.trim());
        if !std::path::Path::new(&file).is_file() {
            return Err(format!("File not found: {file}"));
        }
        let pages = self.pages.trim();
        let job = |pages: Option<String>, reverse| {
            lp_args(&Job { printer, file: &file, color: self.color, paper: PAPERS[self.paper], pages, reverse })
        };
        if !self.duplex {
            let range = (!pages.is_empty() && pages != "all").then(|| pages.to_string());
            return submit(&job(range, false));
        }
        let total = page_count(&file).ok_or("Double-sided needs a PDF file")?;
        let (front, back) = split_duplex(&parse_ranges(pages, total)?);
        let front_job = job(Some(join(&front)), false);
        let back_job = job(Some(join(&back)), self.reverse_back);
        let id = submit(&front_job)?;
        if back.is_empty() {
            return Ok(format!("Only one page, nothing to flip: {id}"));
        }
        self.mode = Mode::Flip(back_job);
        Ok(format!(
            "Front side sent: {id}\n\nWhen printing finishes, flip the stack and put it back in the tray.{}\n\nEnter = print back side    Esc = cancel",
            if front.len() > back.len() { "\nRemove the last printed sheet first, it has no back side." } else { "" }
        ))
    }
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
            let area = popup(f, 9);
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
