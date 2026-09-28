mod i18n;
mod pet;
mod sound;

use i18n::{fill, t};
use pet::{theme, Act, Mood, Work, ACCENT, RED, WHITE, YELLOW};
use sound::Sound;
use printertui::*;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap},
    DefaultTerminal, Frame,
};
use std::cell::Cell;
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Result of a background task, applied to the app on the UI thread.
type Done = Box<dyn FnOnce(&mut App) + Send>;

/// Rows of the Print, Scan and Settings forms.
const ROWS: usize = 13;
const SCAN_ROWS: usize = 10;
const SETTINGS_ROWS: usize = 6;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Print,
    Scan,
    Settings,
}

const TABS: [Tab; 3] = [Tab::Print, Tab::Scan, Tab::Settings];

// Settings rows
const LANG: usize = 0;
const THEME: usize = 1;
const VOLUME: usize = 2;
const MASCOT: usize = 3;
const GRAPHICS: usize = 4;
const FOLDER: usize = 5;
/// Image preview choices as the settings file names them.
const GRAPHICS_PREFS: [&str; 4] = ["auto", "kitty", "sixel", "blocks"];
const FILE: usize = 1;
const PAGES: usize = 5;
const SCALE: usize = 9;
const PRINT: usize = 10;
const ADD: usize = 11;
const QUEUE: usize = 12;

const SAVE_AS: usize = 4;
const SCAN: usize = 5;
const PAGE: usize = 6;
const SAVE: usize = 7;
const COPY: usize = 8;
const DISCARD: usize = 9;

/// A scanned page: edits are always applied to the original scan.
struct ScannedPage {
    orig: String,
    /// The original, or the edited copy once `rot` / `filter` are applied.
    file: String,
    rot: u16,
    filter: usize,
    keep: bool,
    thumb: Thumb,
}

type Thumb = (usize, usize, Vec<u8>);

/// What the Print tab preview shows: the n-th sheet side of the first file as it will print.
#[derive(Clone, PartialEq)]
struct ViewKey {
    file: String,
    pages: String,
    scale: u32,
    paper: &'static str,
    per_sheet: u32,
    idx: usize,
}

/// A rendered Print tab preview sheet side.
struct PrintView {
    png: String,
    thumb: Thumb,
    /// Which sheet side, and how many there are.
    idx: usize,
    count: usize,
    title: String,
}

enum Mode {
    Main,
    /// Editing the selected text field (vim-style, entered with `i`).
    Insert,
    Pick(Vec<(String, String)>, ListState),
    /// Typing a printer's address when none were found on the network.
    Address(String),
    /// Page selector: one checkbox per page of the first file.
    Pages(Vec<bool>, ListState),
    /// Manual duplex: `job` is the front job while it is still printing, `steps` the flip
    /// instructions shown after it, `back` the back-side job, `files[next..]` the files still to print.
    Flip { job: Option<String>, steps: String, back: Job, files: Vec<String>, next: usize },
    /// Print queue popup: (job number, description), refreshed every second.
    Queue(Vec<(String, String)>, ListState),
}

struct App {
    printers: Vec<String>,
    labels: Vec<String>,
    /// The last known state of a printer (by queue), the check running, and when it started.
    state: Option<(String, PrinterState)>,
    checking: Option<mpsc::Receiver<Done>>,
    checked: Instant,
    printer: usize,
    file: String,
    color: bool,
    duplex: bool,
    reverse_back: bool,
    pages: String,
    paper: usize,
    copies: u32,
    per_sheet: usize,
    scale: usize,
    tab: Tab,
    /// Index in pet::THEMES.
    theme: usize,
    mascot: bool,
    /// Index in GRAPHICS_PREFS.
    graphics_pref: usize,
    /// Where scans are saved by default; empty for ~/Documents.
    scan_folder: String,
    /// Print tab preview for its key (or why there is none), the render running, and the page asked for.
    view: Option<(ViewKey, Result<PrintView, String>)>,
    viewing: Option<(ViewKey, mpsc::Receiver<Done>)>,
    view_page: usize,
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
    /// A scan is running (the printer shows its scan light).
    scanning: bool,
    /// Redraw the whole screen: another language can leave letters of different widths behind.
    clear: bool,
    /// Language from the settings ("es"), empty to follow the system's.
    lang: String,
    /// Page rotate/filter re-render, apart from `busy` so pages can be edited while the next one scans.
    editing: Option<mpsc::Receiver<Done>>,
    /// Graphics protocol supported by the terminal: Kitty, Sixel, or None.
    graphics: Graphics,
    /// Size of the preview panel at the last draw, and the image sent to the terminal for it.
    preview_area: Cell<ratatui::layout::Rect>,
    /// Where the form and the open popup were drawn, to map mouse clicks to rows.
    form_area: Cell<ratatui::layout::Rect>,
    popup_area: Cell<ratatui::layout::Rect>,
    sent: Option<(String, u16, u16)>,
    started: Instant,
    last_poll: Instant,
    sel: usize,
    status: String,
    /// The status the printer last reacted to (an error jams it).
    heard: String,
    /// What the printer is doing.
    act: Act,
    mode: Mode,
}

fn main() -> std::io::Result<()> {
    let mut app = App {
        printers: Vec::new(),
        labels: Vec::new(),
        state: None,
        checking: None,
        checked: Instant::now(),
        printer: 0,
        file: std::env::args().nth(1).unwrap_or_default(),
        color: false,
        duplex: false,
        reverse_back: false,
        pages: String::new(),
        paper: 0,
        copies: 1,
        per_sheet: 0,
        scale: SCALES.iter().position(|s| *s == 100).unwrap_or(0),
        tab: if std::env::args().any(|a| a == "--scan") { Tab::Scan } else { Tab::Print },
        theme: 0,
        mascot: true,
        graphics_pref: 0,
        scan_folder: String::new(),
        view: None,
        viewing: None,
        view_page: 0,
        scanners: None,
        scanner: 0,
        scanner_pref: String::new(),
        scan_mode: 0,
        scan_dpi: 1,
        scan_format: 0,
        save_as: String::new(),
        scans: Vec::new(),
        cur: 0,
        busy: None,
        scanning: false,
        clear: false,
        lang: String::new(),
        editing: None,
        graphics: detect_graphics(),
        preview_area: Cell::default(),
        form_area: Cell::default(),
        popup_area: Cell::default(),
        sent: None,
        started: Instant::now(),
        last_poll: Instant::now(),
        sel: 0,
        status: String::new(),
        heard: String::new(),
        act: Act::Idle,
        mode: Mode::Main,
    };
    app.set_printers(printers());
    if let Some(text) = config_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        for (k, v) in parse_config(&text) {
            app.apply(k, v);
        }
    }
    // set by the installer from the language picked in its menu
    app.apply_settings();
    app.save_as = default_scan_name(&app.scan_folder);
    sound::play(Sound::Boot);
    if app.printers.is_empty() {
        app.status = t().no_printers.into();
    }
    // If launched with --scan, preload the most recent scanned page for preview testing
    if app.tab == Tab::Scan {
        let dir = std::env::temp_dir().join("printertui-scan");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut paths: Vec<_> = rd
                .flatten()
                .filter(|e| {
                    let n = e.file_name();
                    let s = n.to_string_lossy();
                    // scans have no extension; edited copies and previews are .png
                    s.starts_with("page-") && !s.contains('.')
                })
                .collect();
            paths.sort_by_key(|e| e.file_name());
            if let Some(entry) = paths.last() {
                let path = entry.path().to_string_lossy().into_owned();
                if let Ok(thumb) = thumbnail(&path) {
                    app.scans.push(ScannedPage { orig: path.clone(), file: path, rot: 0, filter: 0, keep: true, thumb });
                    app.status = t().loaded_last.into();
                }
            }
        }
    }

    let mut term = init();
    let res = run(&mut term, &mut app);
    if let (Graphics::Kitty(tmux), Some(_)) = (app.graphics, &app.sent) {
        emit(&kitty(&format!("a=d,d=I,i={},q=2", image_id()), "", tmux));
    }
    restore();
    stop_all();
    res
}

/// Mouse clicks on Windows, where people expect them; elsewhere the terminal keeps its own text selection.
const MOUSE: bool = cfg!(windows);

fn init() -> DefaultTerminal {
    let term = ratatui::init();
    if MOUSE {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::event::EnableMouseCapture);
    }
    term
}

fn restore() {
    if MOUSE {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::event::DisableMouseCapture);
    }
    ratatui::restore();
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
        if let Some(done) = app.viewing.as_ref().and_then(|(_, rx)| finished(rx)) {
            app.viewing = None;
            done(app);
        }
        if let Some(done) = app.checking.as_ref().and_then(finished) {
            app.checking = None;
            done(app);
        }
        app.check_printer();
        app.sync_view();
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
        if app.status != app.heard {
            app.heard = app.status.clone();
            if app.status.starts_with(t().error) {
                app.act = Act::Jam(Instant::now());
                sound::play(Sound::Jam);
            }
        }
        POPUP.set(Default::default());
        if std::mem::take(&mut app.clear) {
            term.clear()?;
            app.sent = None;
        }
        term.draw(|f| draw(f, app))?;
        app.popup_area.set(POPUP.get());
        app.sync_image();
        // short timeout keeps the spinner and the duplex animation moving
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        // a mouse click becomes the keys it stands for
        let keys = match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => vec![k],
            Event::Mouse(m) => app.mouse(m),
            _ => continue,
        };
        if app.act.done() {
            app.act = Act::Idle;
        }
        for k in keys {
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
                    KeyCode::Char(c @ ('H' | 'L')) if app.tab == Tab::Scan && !app.scans.is_empty() => {
                        let n = app.scans.len();
                        app.cur = (app.cur + if c == 'L' { 1 } else { n - 1 }) % n;
                    }
                    // H/L browse the preview pages on the Print tab
                    KeyCode::Char(c @ ('H' | 'L')) if app.tab == Tab::Print => {
                        if let Some((_, Ok(v))) = &app.view {
                            app.view_page = (v.idx + if c == 'L' { 1 } else { v.count - 1 }) % v.count;
                        }
                    }
                    KeyCode::Char(c) if app.tab == Tab::Scan && app.sel == PAGE && !app.scans.is_empty() && app.page_key(c, dd) => {}
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                    KeyCode::Up | KeyCode::Char('k') => app.sel = app.sel.checked_sub(1).unwrap_or(app.last()),
                    KeyCode::Down | KeyCode::Char('j') => app.sel = (app.sel + 1) % (app.last() + 1),
                    KeyCode::Char('g') if gg => app.sel = 0,
                    KeyCode::Char('G') => app.sel = app.last(),
                    KeyCode::Tab | KeyCode::BackTab => {
                        let i = TABS.iter().position(|t| *t == app.tab).unwrap_or(0);
                        app.tab = TABS[(i + if k.code == KeyCode::Tab { 1 } else { TABS.len() - 1 }) % TABS.len()];
                        app.sent = None;
                        app.sel = 0;
                        app.status = String::new();
                        if app.tab == Tab::Scan && app.scanners.is_none() {
                            app.find_scanners(false);
                        }
                    }
                    KeyCode::Left | KeyCode::Char('h') => app.cycle(false),
                    KeyCode::Right | KeyCode::Char('l') => app.cycle(true),
                    KeyCode::Char('i') | KeyCode::Char('a') if app.text().is_some() => app.mode = Mode::Insert,
                    KeyCode::Enter if app.tab == Tab::Scan => app.scan_enter(),
                    KeyCode::Enter if app.tab == Tab::Settings && app.sel == FOLDER => app.mode = Mode::Insert,
                    KeyCode::Enter if app.tab == Tab::Settings => app.cycle(true),
                    KeyCode::Enter if app.sel == ADD => app.spawn(t().searching_printers, || {
                        let found = discover();
                        Box::new(move |app: &mut App| {
                            if found.is_empty() {
                                app.status = t().no_net_printers.into();
                                app.mode = Mode::Address(String::new());
                            } else {
                                app.status = String::new();
                                app.mode = Mode::Pick(found, ListState::default().with_selected(Some(0)));
                            }
                        })
                    }),
                    KeyCode::Enter if app.sel == FILE => {
                        restore();
                        let picked = pick_files();
                        *term = init();
                        if picked.is_empty() {
                            app.status = fill(t().no_file_picked, &[("hint", &t().hints()[0])]);
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
                    KeyCode::Esc | KeyCode::Enter => {
                        app.mode = Mode::Main;
                        if app.tab == Tab::Settings {
                            app.save_as = default_scan_name(&app.scan_folder);
                            app.save_settings();
                        }
                    }
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
                    KeyCode::Char('a') => app.mode = Mode::Address(String::new()),
                    KeyCode::Enter => {
                        let (name, uri) = found[state.selected().unwrap_or(0)].clone();
                        app.add(term, &name, &uri);
                    }
                    _ => {}
                },
                Mode::Address(addr) => match k.code {
                    KeyCode::Esc => app.mode = Mode::Main,
                    KeyCode::Backspace => {
                        addr.pop();
                    }
                    KeyCode::Char(c) => addr.push(c),
                    KeyCode::Enter if !addr.trim().is_empty() => {
                        let addr = addr.trim().to_string();
                        // a bare IP or name gets the standard IPP Everywhere path; a full uri is used as is
                        let uri = if addr.contains("://") { addr.clone() } else { format!("ipp://{addr}/ipp/print") };
                        let host = uri_host(&uri).map_or(addr.clone(), |(_, h)| h.to_string());
                        app.add(term, &queue_name(&host), &uri);
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
                                app.status = t().select_a_page.into();
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
                                Ok(()) => fill(t().cancelled, &[("desc", &desc)]),
                                Err(e) => failed(e),
                            };
                            *jobs = queue();
                        }
                    }
                    _ => {}
                },
                Mode::Flip { job, .. } => match k.code {
                    KeyCode::Esc => {
                        app.status = t().back_cancelled.into();
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
        Err(mpsc::TryRecvError::Disconnected) => Some(Box::new(|app: &mut App| app.status = failed(t().task_crashed))),
        Err(mpsc::TryRecvError::Empty) => None,
    }
}

impl App {
    fn spawn(&mut self, msg: impl Into<String>, work: impl FnOnce() -> Done + Send + 'static) {
        self.busy = Some((msg.into(), task(work)));
    }

    /// The keys a mouse event stands for: a click selects a row (or popup item), a click on `<` / `>`
    /// changes the value, a click on a button, File or Pages (or on the selected popup item) opens it,
    /// and the wheel moves up and down.
    fn mouse(&mut self, m: MouseEvent) -> Vec<KeyEvent> {
        let key = |code| vec![KeyEvent::new(code, KeyModifiers::NONE)];
        match m.kind {
            MouseEventKind::ScrollUp => return key(KeyCode::Up),
            MouseEventKind::ScrollDown => return key(KeyCode::Down),
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return Vec::new(),
        }
        let (x, y) = (m.column, m.row);
        let popup = self.popup_area.get();
        let item = (popup.contains((x, y).into()) && y > popup.y).then(|| (y - popup.y - 1) as usize);
        match &mut self.mode {
            Mode::Pick(list, state) | Mode::Queue(list, state) => {
                let Some(i) = item.filter(|i| *i < list.len()) else { return Vec::new() };
                let again = state.selected() == Some(i);
                state.select(Some(i));
                // a second click adds the printer; jobs are only cancelled with x
                return if again && matches!(self.mode, Mode::Pick(..)) { key(KeyCode::Enter) } else { Vec::new() };
            }
            Mode::Pages(on, state) => {
                let Some(i) = item.filter(|i| *i < on.len()) else { return Vec::new() };
                state.select(Some(i));
                return key(KeyCode::Char(' '));
            }
            Mode::Flip { .. } => return key(KeyCode::Enter),
            Mode::Insert | Mode::Address(_) => return Vec::new(),
            Mode::Main => {}
        }
        let form = self.form_area.get();
        // the tabs in the title after " PrinterTUI  ": a click on one is as many Tab presses
        if y == form.y {
            let mut left = form.x + 14;
            for (i, name) in [t().tab_print, t().tab_scan, t().tab_settings].iter().enumerate() {
                let right = left + width(name) as u16 + 2;
                if (left..right).contains(&x) {
                    let now = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
                    return vec![KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE); (i + TABS.len() - now) % TABS.len()];
                }
                left = right;
            }
            return Vec::new();
        }
        let rows = self.rows();
        let Some(row) = y.checked_sub(form.y + 1).map(usize::from).filter(|r| *r < rows && form.contains((x, y).into())) else {
            return Vec::new();
        };
        self.sel = row;
        let value = self.value(row);
        let Some(c) = x.checked_sub(form.x + value_col()).and_then(|i| char_at(&value, i as usize)) else { return Vec::new() };
        match c {
            '<' => key(KeyCode::Left),
            '>' => key(KeyCode::Right),
            _ if value.starts_with('[') => key(KeyCode::Enter),
            _ if self.tab == Tab::Print && (row == FILE || row == PAGES) => key(KeyCode::Enter),
            _ if self.tab == Tab::Settings && row == FOLDER => key(KeyCode::Enter),
            '[' | 'x' | ']' if self.tab == Tab::Scan && row == PAGE => key(KeyCode::Char('x')),
            _ => Vec::new(),
        }
    }

    /// Adds a network printer; the terminal is in normal mode meanwhile, for the password / UAC prompt.
    fn add(&mut self, term: &mut DefaultTerminal, name: &str, uri: &str) {
        restore();
        println!("{}", fill(t().adding, &[("name", &name), ("uri", &uri), ("note", &t().hints()[2])]));
        let res = add_printer(name, uri);
        *term = init();
        self.status = match res {
            Ok(()) => fill(t().added, &[("name", &name)]),
            Err(e) => e,
        };
        self.set_printers(printers());
        self.printer = self.printers.iter().position(|p| p == name).unwrap_or(0);
        self.mode = Mode::Main;
    }

    fn set_printers(&mut self, printers: Vec<String>) {
        self.labels = printer_labels(&printers);
        self.printers = printers;
    }

    fn rows(&self) -> usize {
        match self.tab {
            Tab::Print => ROWS,
            Tab::Scan => SCAN_ROWS,
            Tab::Settings => SETTINGS_ROWS,
        }
    }

    fn last(&self) -> usize {
        self.rows() - 1
    }

    fn text(&mut self) -> Option<&mut String> {
        match self.tab {
            Tab::Scan => return (self.sel == SAVE_AS).then_some(&mut self.save_as),
            Tab::Settings => return (self.sel == FOLDER).then_some(&mut self.scan_folder),
            Tab::Print => {}
        }
        match self.sel {
            FILE => Some(&mut self.file),
            PAGES => Some(&mut self.pages),
            _ => None,
        }
    }

    /// Changes the selected row's value; the printer hops when it does.
    fn cycle(&mut self, fwd: bool) {
        let before = self.value(self.sel);
        self.step_value(fwd);
        if self.value(self.sel) != before {
            self.act = Act::Hop(Instant::now());
            sound::play(Sound::Blip);
            if self.tab == Tab::Settings {
                self.save_settings();
            }
        }
    }

    fn step_value(&mut self, fwd: bool) {
        let step = |i: usize, n: usize| if n == 0 { 0 } else if fwd { (i + 1) % n } else { (i + n - 1) % n };
        if self.tab == Tab::Settings {
            match self.sel {
                LANG => {
                    // 0 follows the system, then the ten languages
                    let now = if self.lang.is_empty() { 0 } else { i18n::find(&self.lang).map_or(0, |i| i + 1) };
                    let next = step(now, i18n::ALL.len() + 1);
                    self.lang = if next == 0 { String::new() } else { i18n::ALL[next - 1].code.to_string() };
                }
                THEME => self.theme = step(self.theme, pet::THEMES.len()),
                VOLUME => sound::set_volume(if fwd { sound::volume().saturating_add(10).min(100) } else { sound::volume().saturating_sub(10) }),
                MASCOT => self.mascot = !self.mascot,
                GRAPHICS => self.graphics_pref = step(self.graphics_pref, GRAPHICS_PREFS.len()),
                _ => {}
            }
            return self.apply_settings();
        }
        if self.tab == Tab::Scan {
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
            SCALE => self.scale = step(self.scale, SCALES.len()),
            _ => {}
        }
    }

    fn value(&self, row: usize) -> String {
        let pick = |s: &str| format!("< {s} >");
        if self.tab == Tab::Settings {
            return match row {
                LANG if self.lang.is_empty() => {
                    let system = i18n::find(&system_language()).unwrap_or(0);
                    pick(&fill(t().system, &[("lang", &i18n::NAMES[system])]))
                }
                LANG => pick(i18n::find(&self.lang).map_or("?", |i| i18n::NAMES[i])),
                THEME => pick(t().themes[self.theme]),
                VOLUME if sound::volume() == 0 => pick(t().muted),
                VOLUME => pick(&format!("{}%", sound::volume())),
                MASCOT => pick(t().on_off[usize::from(!self.mascot)]),
                GRAPHICS => pick(t().graphics[self.graphics_pref]),
                FOLDER if self.scan_folder.is_empty() && !matches!(self.mode, Mode::Insert) => default_scan_folder(),
                _ => self.scan_folder.clone(),
            };
        }
        if self.tab == Tab::Scan {
            return match row {
                0 => pick(self.scanners.as_ref().and_then(|l| l.get(self.scanner)).map_or(t().none, |(_, d)| d.as_str())),
                1 => pick(t().scan_modes[self.scan_mode]),
                2 => pick(&format!("{} dpi", SCAN_DPI[self.scan_dpi])),
                3 => pick(match SCAN_FORMATS[self.scan_format] {
                    "PDF" => t().fmt_pdf,
                    "OCR" => t().fmt_ocr,
                    _ => t().fmt_png,
                }),
                SAVE_AS => self.save_as.clone(),
                SCAN => format!("[ {} ]", t().btn_scan),
                PAGE => match self.scans.get(self.cur) {
                    None => t().none_yet.into(),
                    Some(p) => {
                        let mut v = format!("< {} / {} >  [{}] {}", self.cur + 1, self.scans.len(), if p.keep { "x" } else { " " }, t().keep);
                        if p.rot != 0 {
                            v += &format!(" · {}", fill(t().rotated, &[("deg", &p.rot)]));
                        }
                        if p.filter != 0 {
                            v += &format!(" · {}", t().filters[p.filter]);
                        }
                        v
                    }
                },
                SAVE => {
                    let (kept, all) = (self.scans.iter().filter(|p| p.keep).count(), self.scans.len());
                    let text = match kept {
                        _ if kept < all => fill(t().save_some, &[("n", &kept), ("all", &all)]),
                        1 => t().save_one.into(),
                        n => fill(t().save_many, &[("n", &n)]),
                    };
                    format!("[ {text} ]")
                }
                COPY => match self.scans.iter().filter(|p| p.keep).count() {
                    0 if self.scans.is_empty() => format!("[ {} ]", t().copy_scan),
                    1 => format!("[ {} ]", t().copy_one),
                    n => format!("[ {} ]", fill(t().copy_many, &[("n", &n)])),
                },
                _ => format!("[ {} ]", t().btn_discard),
            };
        }
        match row {
            0 => pick(self.labels.get(self.printer).map_or(t().none, String::as_str)),
            FILE if matches!(self.mode, Mode::Insert) => self.file.clone(),
            FILE => match split_files(&self.file)[..] {
                [] => String::new(),
                [ref one] => one.clone(),
                ref many => {
                    let names: Vec<&str> = many.iter().map(|f| name(f)).collect();
                    fill(t().files, &[("n", &many.len()), ("names", &names.join(", "))])
                }
            },
            2 => pick(if self.color { t().color } else { t().grayscale }),
            3 => pick(if self.duplex { t().double_sided } else { t().single_sided }),
            4 => pick(if self.reverse_back { t().reversed } else { t().normal }),
            PAGES if self.pages.is_empty() && !matches!(self.mode, Mode::Insert) => t().all.into(),
            PAGES => self.pages.clone(),
            6 => pick(PAPERS[self.paper]),
            7 => pick(&self.copies.to_string()),
            8 => pick(&PER_SHEET[self.per_sheet].to_string()),
            SCALE => pick(&format!("{}%", SCALES[self.scale])),
            PRINT => format!("[ {} ]", t().btn_print),
            ADD => format!("[ {} ]", t().btn_add),
            _ => format!("[ {} ]", t().btn_queue),
        }
    }

    /// Opens the page selector for the first file, pre-checking the current range.
    fn open_pages(&mut self) {
        let Some(file) = split_files(&self.file).first().map(|f| expand_home(f)) else {
            return self.status = failed(t().pick_file_first);
        };
        let paper = PAPERS[self.paper];
        self.spawn(t().reading_pages, move || {
            let total = printable(&file, 100, paper).and_then(|p| page_count(&p).ok_or(t().unreadable_pdf.into()));
            Box::new(move |app: &mut App| match total {
                Ok(total) => {
                    let current = parse_ranges(&app.pages, total).unwrap_or_default();
                    let on = (1..=total).map(|n| current.contains(&n)).collect();
                    app.status = String::new();
                    app.mode = Mode::Pages(on, ListState::default().with_selected(Some(0)));
                }
                Err(e) => app.status = failed(e),
            })
        });
    }

    /// A print's result: pages come out of the printer, or the status says what went wrong.
    fn show(&mut self, res: Result<String, String>) {
        if res.is_ok() {
            self.act = Act::Print(Instant::now(), self.copies.clamp(1, 3));
            sound::play(Sound::Print);
        }
        self.status = match res {
            Ok(s) => match self.save() {
                Ok(()) => s,
                Err(e) => format!("{s}\n\n{}", fill(t().settings_not_saved, &[("e", &e)])),
            },
            Err(e) => failed(e),
        }
    }

    /// Remembers the settings: the Print and Scan ones after a print or scan (not the file or
    /// page range), the Settings tab's as they change.
    fn save(&self) -> std::io::Result<()> {
        let path = config_path().ok_or(std::io::Error::other("HOME is not set"))?;
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
        std::fs::write(path, format!(
            "printer={}\ncolor={}\nduplex={}\nreverse_back={}\npaper={}\ncopies={}\nper_sheet={}\nscale={}\n\
             scanner={}\nscan_mode={}\nscan_dpi={}\nscan_format={}\n\
             lang={}\ntheme={}\nvolume={}\nmascot={}\ngraphics={}\nscan_folder={}\n",
            self.printers.get(self.printer).map_or("", String::as_str),
            self.color, self.duplex, self.reverse_back, PAPERS[self.paper], self.copies, PER_SHEET[self.per_sheet], SCALES[self.scale],
            self.scanners.as_ref().and_then(|l| l.get(self.scanner)).map_or(self.scanner_pref.as_str(), |(d, _)| d.as_str()),
            SCAN_MODES[self.scan_mode], SCAN_DPI[self.scan_dpi], SCAN_FORMATS[self.scan_format],
            // an empty language follows the system's
            self.lang, pet::THEMES[self.theme].0, sound::volume(), self.mascot, GRAPHICS_PREFS[self.graphics_pref], self.scan_folder,
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
            "lang" => self.lang = v.to_string(),
            "theme" => self.theme = pet::THEMES.iter().position(|(name, _)| *name == v).unwrap_or(self.theme),
            "volume" => sound::set_volume(v.parse().unwrap_or(sound::volume())),
            "mascot" => self.mascot = b.unwrap_or(self.mascot),
            "graphics" => self.graphics_pref = GRAPHICS_PREFS.iter().position(|g| *g == v).unwrap_or(self.graphics_pref),
            "scan_folder" => self.scan_folder = v.to_string(),
            "scale" => self.scale = SCALES.iter().position(|n| n.to_string() == v).unwrap_or(self.scale),
            "scanner" => self.scanner_pref = v.to_string(),
            "scan_mode" => self.scan_mode = SCAN_MODES.iter().position(|m| *m == v).unwrap_or(self.scan_mode),
            "scan_dpi" => self.scan_dpi = SCAN_DPI.iter().position(|d| d.to_string() == v).unwrap_or(self.scan_dpi),
            "scan_format" => self.scan_format = SCAN_FORMATS.iter().position(|f| *f == v).unwrap_or(self.scan_format),
            _ => {}
        }
    }


    fn job(&self, file: &str, pages: Option<String>, reverse: bool, collate: bool) -> Job {
        Job {
            printer: self.printers[self.printer].clone(), file: file.into(), color: self.color, paper: PAPERS[self.paper], pages, reverse,
            copies: self.copies, collate, per_sheet: PER_SHEET[self.per_sheet],
        }
    }

    fn files(&self) -> Result<Vec<String>, String> {
        self.printers.get(self.printer).ok_or(t().no_printer_selected)?;
        let files: Vec<String> = split_files(&self.file).iter().map(|f| expand_home(f)).collect();
        if files.is_empty() {
            return Err(t().no_file_selected.into());
        }
        if let Some(f) = files.iter().find(|f| !std::path::Path::new(f).is_file()) {
            return Err(fill(t().file_not_found, &[("file", f)]));
        }
        Ok(files)
    }

    /// Converts the files to PDF in the background, then sends the jobs.
    fn try_print(&mut self) {
        let files = match self.files() {
            Ok(f) => f,
            Err(e) => return self.show(Err(e)),
        };
        let (scale, paper) = (SCALES[self.scale], PAPERS[self.paper]);
        self.spawn(t().preparing, move || {
            let pdfs: Result<Vec<String>, String> = files.iter().map(|f| printable(f, scale, paper)).collect();
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
        let all = pages.is_empty() || pages == "all";
        // checked against each PDF: CUPS takes page-ranges beyond the last page without a word
        let sent: Result<Vec<String>, String> = pdfs
            .iter()
            .map(|p| {
                let range = if all { None } else { Some(join(&parse_ranges(pages, page_count(p).ok_or(t().unreadable_pdf)?)?)) };
                submit(&self.job(p, range, false, true))
            })
            .collect();
        Ok(sent?.join("\n"))
    }

    /// Prints the front of files[i..] until one needs flipping, then waits in Mode::Flip.
    fn duplex(&mut self, files: Vec<String>, mut i: usize, mut done: String) -> Result<String, String> {
        self.mode = Mode::Main;
        while let Some(file) = files.get(i) {
            i += 1;
            let head = if files.len() > 1 { fill(t().file_of, &[("i", &i), ("n", &files.len()), ("name", &name(file))]) + "\n" } else { String::new() };
            let pdf = file.clone();
            let total = page_count(&pdf).ok_or(t().unreadable_pdf)?;
            // page-ranges picks pages before number-up groups them, so split whole sheet sides
            let pages = parse_ranges(&self.pages, total)?;
            let sides: Vec<&[u32]> = pages.chunks(PER_SHEET[self.per_sheet] as usize).collect();
            let (front, back) = split_duplex(&sides);
            // Collated copies would leave a sheet without back side inside every copy when odd.
            let odd = front.len() > back.len();
            let (front, back) = (front.concat(), back.concat());
            let id = submit(&self.job(&pdf, Some(join(&front)), false, !odd))?;
            if back.is_empty() {
                done += &format!("{head}{}\n\n", fill(t().one_page, &[("id", &id)]));
                continue;
            }
            let aside = match (odd, self.copies) {
                (false, _) => String::new(),
                (true, 1) => t().aside_one.into(),
                (true, n) => fill(t().aside_many, &[("n", &n)]),
            };
            let sent = fill(t().front_sent, &[("id", &id)]);
            let steps = format!("{done}{head}{sent}\n\n{}", fill(t().flip_steps, &[("aside", &aside)]));
            let wait = format!("{done}{head}{sent}\n\n{}", t().printing_front);
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
        self.duplex(files, next, fill(t().back_sent, &[("id", &id)]) + "\n\n")
    }
}

impl App {
    /// Sends the scanned page preview to the terminal using Kitty graphics or Sixel protocol.
    fn sync_image(&mut self) {
        match self.graphics {
            Graphics::Kitty(tmux) => {
                let area = self.preview_area.get();
                let want = self.preview_png().map(|p| (p, area.width, area.height));
                if want.is_some() && want != self.sent && area.width > 0 {
                    let (png, c, r) = want.clone().unwrap_or_default();
                    emit(&kitty(&format!("a=T,U=1,f=100,t=f,i={},q=2,c={c},r={r}", image_id()), &base64(png.as_bytes()), tmux));
                    self.sent = want;
                }
            }
            Graphics::Sixel => {
                let area = self.preview_area.get();
                let clear = |buf: &mut String| {
                    for r in 0..area.height {
                        buf.push_str(&format!("\x1b[{};{}H{:w$}", area.y + 1 + r, area.x + 1, " ", w = area.width as usize));
                    }
                };
                let Some((png, (w, h, px))) = self.shown() else {
                    // wipe the last image, ratatui does not know it is there
                    if self.sent.take().is_some() {
                        let mut buf = String::new();
                        clear(&mut buf);
                        emit(&buf);
                    }
                    return;
                };
                let want = Some((png, area.width, area.height));
                if want != self.sent && area.width > 0 && area.height > 0 {
                    let max_w = (area.width as usize).saturating_sub(1) * 10;
                    let max_h = (area.height as usize) * 18;
                    if max_w > 0 && max_h > 0 && *w > 0 && *h > 0 {
                        let scale = (max_w as f32 / *w as f32).min(max_h as f32 / *h as f32);
                        let ow = ((*w as f32 * scale) as usize).clamp(10, max_w);
                        let oh = (((((*h as f32 * scale) as usize) + 5) / 6 * 6).max(6)).min(max_h / 6 * 6);
                        if ow > 0 && oh > 0 {
                            let small = downscale(*w, *h, px, ow, oh);
                            let sixel = sixel_encode(ow, oh, &small);
                            let mut buf = String::new();
                            clear(&mut buf);
                            let cols_used = ((ow as f32 / 10.0).ceil() as u16).min(area.width);
                            let offset_x = (area.width.saturating_sub(cols_used)) / 2;
                            let x = area.x + 1 + offset_x;
                            let y = area.y + 1;
                            buf.push_str(&format!("\x1b[{y};{x}H{sixel}\x1b[?25l"));
                            emit(&buf);
                        }
                    }
                    self.sent = want;
                }
            }
            Graphics::None => {}
        }
    }

    /// What the printer says about the selected row when there is no news.
    fn chat(&self) -> String {
        let n = |template: &str, n: &dyn std::fmt::Display| fill(template, &[("n", n)]);
        if self.tab == Tab::Settings {
            return match self.sel {
                LANG => t().lang_chat.into(),
                THEME => t().theme_chat.into(),
                VOLUME if sound::volume() == 0 => t().muted_chat.into(),
                VOLUME => t().volume_chat.into(),
                MASCOT => t().mascot_chat.into(),
                GRAPHICS => t().graphics_chat.into(),
                _ => t().folder_chat.into(),
            };
        }
        if self.tab == Tab::Scan {
            let kept = self.scans.iter().filter(|p| p.keep).count();
            return match self.sel {
                0 if self.scanners.as_ref().is_some_and(Vec::is_empty) => t().no_scanner_yet.into(),
                0 => t().search_again.into(),
                1 => fill(t().mode_chat, &[("mode", &t().scan_modes[self.scan_mode])]),
                2 if SCAN_DPI[self.scan_dpi] >= 600 => n(t().dpi_high, &SCAN_DPI[self.scan_dpi]),
                2 => n(t().dpi_chat, &SCAN_DPI[self.scan_dpi]),
                3 if SCAN_FORMATS[self.scan_format] == "OCR" => t().ocr_chat.into(),
                3 => t().format_chat.into(),
                SAVE_AS => t().save_as_chat.into(),
                SCAN => t().scan_chat.into(),
                PAGE if self.scans.is_empty() => t().pages_here.into(),
                PAGE => t().page_keys.into(),
                SAVE if kept == 0 => t().nothing_to_save.into(),
                SAVE if kept == 1 => t().ready_one.into(),
                SAVE => n(t().ready_many, &kept),
                COPY if self.scans.is_empty() => t().copy_chat.into(),
                COPY if kept == 1 => t().copy_one_chat.into(),
                COPY => n(t().copy_many_chat, &kept),
                _ => t().discard_chat.into(),
            };
        }
        match self.sel {
            0 if self.printers.is_empty() => t().no_printer_yet.into(),
            0 if self.printer_state().is_some_and(|s| s.ink.iter().any(|i| i.2)) => t().ink_low.into(),
            0 => fill(t().at_service, &[("name", &self.labels.get(self.printer).map_or(t().your_printer, String::as_str))]),
            FILE if self.file.is_empty() => t().feed_me.into(),
            FILE => t().other_files.into(),
            2 if self.color => t().colors.into(),
            2 => t().grayscale_chat.into(),
            3 if self.duplex => t().duplex_chat.into(),
            3 => t().one_side.into(),
            4 => t().back_order_chat.into(),
            PAGES => t().pages_chat.into(),
            6 => fill(t().paper_chat, &[("paper", &PAPERS[self.paper])]),
            7 if self.copies >= 10 => n(t().copies_lots, &self.copies),
            7 if self.copies == 1 => t().copies_one.into(),
            7 => n(t().copies_many, &self.copies),
            8 if PER_SHEET[self.per_sheet] > 1 => n(t().per_sheet_many, &PER_SHEET[self.per_sheet]),
            8 => t().per_sheet_one.into(),
            SCALE => match SCALES[self.scale] {
                100 => t().scale_same.into(),
                s if s > 100 => n(t().scale_big, &s),
                s => n(t().scale_small, &s),
            },
            PRINT => t().print_chat.into(),
            ADD => t().add_chat.into(),
            _ => t().queue_chat.into(),
        }
    }

    fn preview_png(&self) -> Option<String> {
        self.shown().map(|(png, _)| png)
    }

    /// Asks the selected printer how it is doing: when it changes, and every 20 seconds.
    fn check_printer(&mut self) {
        let Some(queue) = self.printers.get(self.printer).cloned() else { return };
        let known = self.state.as_ref().is_some_and(|(q, _)| *q == queue);
        if self.checking.is_some() || (known && self.checked.elapsed() < Duration::from_secs(20)) {
            return;
        }
        self.checked = Instant::now();
        self.checking = Some(task(move || {
            let state = printer_state(&queue);
            Box::new(move |app: &mut App| app.state = Some((queue, state)))
        }));
    }

    /// The selected printer's last known state.
    fn printer_state(&self) -> Option<&PrinterState> {
        self.state.as_ref().filter(|(q, _)| self.printers.get(self.printer) == Some(q)).map(|(_, s)| s)
    }

    /// How the printer looks while nothing else happens, from its worst problem.
    fn mood(&self) -> Mood {
        let problems = self.printer_state().map_or(&[][..], |s| s.problems.as_slice());
        let has = |names: &[&str]| problems.iter().any(|p| names.contains(&p.as_str()));
        if has(&["offline", "shutdown", "connecting-to-device", "timed-out", "stopped", "paused"]) {
            Mood::Off
        } else if has(&["media-empty", "media-needed"]) {
            Mood::NoPaper
        } else if problems.is_empty() {
            Mood::Fine
        } else {
            Mood::Trouble
        }
    }

    /// What the printer says about its worst problem, if it has one.
    fn complaint(&self) -> Option<String> {
        let problem = self.printer_state()?.problems.first()?;
        Some(match problem.as_str() {
            "media-empty" | "media-needed" => t().no_paper.into(),
            "media-jam" => t().jam.into(),
            "door-open" | "cover-open" | "interlock-open" => t().door_open.into(),
            "marker-supply-empty" | "toner-empty" => t().no_ink.into(),
            "offline" | "shutdown" | "connecting-to-device" | "timed-out" => t().offline.into(),
            "stopped" | "paused" => t().paused.into(),
            "output-area-full" => t().tray_full.into(),
            "input-tray-missing" => t().tray_missing.into(),
            other => fill(t().problem, &[("reason", &other)]),
        })
    }

    /// The image in the preview panel: its full-quality PNG and its thumbnail.
    fn shown(&self) -> Option<(String, &Thumb)> {
        match (self.tab, &self.view) {
            (Tab::Scan, _) => self.scans.get(self.cur).map(|p| (format!("{}.preview.png", p.file), &p.thumb)),
            (Tab::Print, Some((_, Ok(v)))) => Some((format!("{}.preview.png", v.png), &v.thumb)),
            _ => None,
        }
    }

    /// Puts the language, theme, volume and image preview settings into effect.
    fn apply_settings(&mut self) {
        let before = t().code;
        i18n::set(if self.lang.is_empty() { system_language() } else { self.lang.clone() }.as_str());
        self.clear |= t().code != before;
        pet::set_theme(self.theme);
        let graphics = match GRAPHICS_PREFS[self.graphics_pref] {
            "kitty" => Graphics::Kitty(std::env::var_os("TMUX").is_some()),
            "sixel" => Graphics::Sixel,
            "blocks" => Graphics::None,
            _ => detect_graphics(),
        };
        if graphics != self.graphics {
            // the old image stays on screen otherwise
            if let (Graphics::Kitty(tmux), Some(_)) = (self.graphics, &self.sent) {
                emit(&kitty(&format!("a=d,d=I,i={},q=2", image_id()), "", tmux));
            }
            self.graphics = graphics;
            self.sent = None;
        }
    }

    /// Saves the settings right away, as the Settings tab changes them.
    fn save_settings(&mut self) {
        if let Err(e) = self.save() {
            self.status = fill(t().settings_not_saved, &[("e", &e)]);
        }
    }

    /// Renders the Print tab preview in the background whenever what it should show changes.
    fn sync_view(&mut self) {
        if self.tab != Tab::Print || matches!(self.mode, Mode::Insert) {
            return;
        }
        let Some(file) = split_files(&self.file).first().map(|f| expand_home(f)) else {
            self.view = None;
            return;
        };
        let key = ViewKey {
            file,
            pages: self.pages.clone(),
            scale: SCALES[self.scale],
            paper: PAPERS[self.paper],
            per_sheet: PER_SHEET[self.per_sheet],
            idx: self.view_page,
        };
        if self.view.as_ref().is_some_and(|(k, _)| *k == key) || self.viewing.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let k = key.clone();
        // a newer render replaces a running one: dropping its receiver discards the result
        self.viewing = Some((key, task(move || {
            let res = render_view(&k);
            Box::new(move |app: &mut App| app.view = Some((k, res)))
        })));
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
                self.status = fill(t().page_deleted, &[("n", &(i + 1))]);
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
                Err(e) => app.status = failed(e),
            })
        }));
    }

    fn find_scanners(&mut self, full: bool) {
        self.spawn(t().searching_scanners, move || {
            let list = scanners(full);
            Box::new(move |app: &mut App| {
                app.scanner = list.iter().position(|(d, _)| *d == app.scanner_pref).unwrap_or(0);
                app.status = if list.is_empty() {
                    fill(t().no_scanners, &[("hint", &t().hints()[1])])
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
            SCAN => self.scan_page(false),
            SAVE => self.save_scans(),
            COPY => self.copy(),
            DISCARD => {
                self.scans.clear();
                self.cur = 0;
                self.status = t().discarded.into();
            }
            _ => {}
        }
    }

    /// Scans a page; `then_copy` prints it right away (Copy with nothing scanned yet).
    fn scan_page(&mut self, then_copy: bool) {
        let Some((device, _)) = self.scanners.as_ref().and_then(|l| l.get(self.scanner)).cloned() else {
            return self.status = failed(t().no_scanner_selected);
        };
        let dir = std::env::temp_dir().join("printertui-scan");
        let n = self.scans.len() + 1;
        // unique name: pages can be deleted and reordered, and old edits must not be reused
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let path = dir.join(format!("page-{stamp}")).to_string_lossy().into_owned();
        let (mode, dpi) = (SCAN_MODES[self.scan_mode], SCAN_DPI[self.scan_dpi]);
        self.scanning = true;
        sound::play(Sound::Scan);
        self.spawn(fill(t().scanning, &[("n", &n)]), move || {
            let res = std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .and_then(|_| scan(&device, mode, dpi, &path))
                .and_then(|_| thumbnail(&path));
            Box::new(move |app: &mut App| {
                app.scanning = false;
                app.status = match res {
                    Ok(thumb) => {
                        app.scans.push(ScannedPage { orig: path.clone(), file: path, rot: 0, filter: 0, keep: true, thumb });
                        app.cur = app.scans.len() - 1;
                        sound::play(Sound::Done);
                        if then_copy {
                            return app.copy();
                        }
                        fill(t().scanned, &[("n", &n)])
                    }
                    Err(e) => failed(e),
                }
            })
        });
    }

    /// Photocopy: prints the kept pages at their real size with the Print tab's printer, color,
    /// paper, copies and scale; with nothing scanned yet it scans a page first.
    fn copy(&mut self) {
        if self.printers.is_empty() {
            return self.status = failed(t().no_printer_for_copy);
        }
        if self.scans.is_empty() {
            return self.scan_page(true);
        }
        let files: Vec<String> = self.scans.iter().filter(|p| p.keep).map(|p| p.file.clone()).collect();
        if files.is_empty() {
            return self.status = failed(t().no_pages_copy);
        }
        if self.editing.is_some() {
            return self.status = t().edit_running.into();
        }
        let pdf = std::env::temp_dir().join("printertui-scan").join("copy.pdf");
        let (dpi, scale, paper) = (SCAN_DPI[self.scan_dpi], SCALES[self.scale], PAPERS[self.paper]);
        let job = self.job("", None, false, true);
        self.spawn(t().printing_copy, move || {
            let res = images_to_pdf(&files, dpi, &[])
                .and_then(|data| std::fs::write(&pdf, data).map_err(|e| format!("{}: {e}", pdf.display())))
                .and_then(|_| printable(&pdf.to_string_lossy(), scale, paper))
                .and_then(|file| submit(&Job { file, ..job }));
            Box::new(move |app: &mut App| app.show(res.map(|id| fill(t().copy_sent, &[("id", &id)]))))
        });
    }

    fn save_scans(&mut self) {
        let files: Vec<String> = self.scans.iter().filter(|p| p.keep).map(|p| p.file.clone()).collect();
        if files.is_empty() {
            return self.status = failed(t().no_pages_save);
        }
        if self.editing.is_some() {
            return self.status = t().edit_running.into();
        }
        let out = expand_home(self.save_as.trim());
        let out = out.trim_end_matches(".pdf").trim_end_matches(".png").to_string();
        let (format, dpi) = (SCAN_FORMATS[self.scan_format], SCAN_DPI[self.scan_dpi]);
        self.spawn(t().saving, move || {
            let res = save_scans(&files, &out, format, dpi);
            Box::new(move |app: &mut App| {
                app.status = match res {
                    Ok(written) => {
                        app.scans.clear();
                        app.cur = 0;
                        app.save_as = default_scan_name(&app.scan_folder);
                        let _ = app.save();
                        sound::play(Sound::Done);
                        let written: Vec<String> = written.iter().map(|p| tilde(p)).collect();
                        fill(t().saved, &[("files", &written.join(", "))])
                    }
                    Err(e) => failed(e),
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Graphics {
    None,
    Kitty(bool),
    Sixel,
}

/// Kitty graphics protocol support: kitty and Ghostty, directly or through tmux when
/// `allow-passthrough` is on. Returns Some(inside tmux).
fn kitty_graphics() -> Option<bool> {
    let var = |k| std::env::var(k).unwrap_or_default();
    let known = |term: &str| {
        let t = term.to_ascii_lowercase();
        t.contains("kitty") || t.contains("ghostty")
    };
    if std::env::var_os("TMUX").is_some() {
        let tmux = |args: &[&str]| std::process::Command::new("tmux").args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let outer = tmux(&["display", "-p", "#{client_termname}"]).unwrap_or_default();
        let passthrough = tmux(&["show", "-gv", "allow-passthrough"]).unwrap_or_default();
        return (known(&outer) && (passthrough == "on" || passthrough == "all")).then_some(true);
    }
    (known(&var("TERM")) || known(&var("TERM_PROGRAM")) || !var("KITTY_WINDOW_ID").is_empty()).then_some(false)
}

fn detect_graphics() -> Graphics {
    let var = |k| std::env::var(k).unwrap_or_default().to_ascii_lowercase();
    let pref = var("PRINTERTUI_GRAPHICS");
    if pref == "sixel" {
        return Graphics::Sixel;
    }
    if pref == "kitty" {
        return Graphics::Kitty(std::env::var_os("TMUX").is_some());
    }
    if pref == "none" {
        return Graphics::None;
    }
    if let Some(tmux) = kitty_graphics() {
        return Graphics::Kitty(tmux);
    }
    let term = var("TERM");
    let prog = var("TERM_PROGRAM");
    if std::env::var_os("WT_SESSION").is_some()
        || std::env::var_os("WT_PROFILE_ID").is_some()
        || term.contains("sixel")
        || term.contains("foot")
        || prog.contains("mintty")
        || prog.contains("contour")
        || prog.contains("wezterm")
    {
        return Graphics::Sixel;
    }
    Graphics::None
}

/// Where scans go unless the settings say otherwise: ~/Documents, or ~ without one.
fn default_scan_folder() -> String {
    let docs = std::env::home_dir().is_some_and(|h| h.join("Documents").is_dir());
    if docs { "~/Documents".into() } else { "~".into() }
}

/// `~/Documents/scan-2026-09-26_154200`, in the folder from the settings when there is one.
fn default_scan_name(folder: &str) -> String {
    let folder = if folder.trim().is_empty() { default_scan_folder() } else { folder.trim().trim_end_matches(['/', '\\']).to_string() };
    format!("{folder}/scan-{}", timestamp())
}

/// The preview for a key: the file as it will print (a PDF, scaled), one sheet side of its range.
fn render_view(k: &ViewKey) -> Result<PrintView, String> {
    if !std::path::Path::new(&k.file).is_file() {
        return Err(fill(t().file_not_found, &[("file", &k.file)]));
    }
    let pdf = printable(&k.file, k.scale, k.paper)?;
    let total = page_count(&pdf).ok_or(t().unreadable_pdf)?;
    let pages = parse_ranges(&k.pages, total)?;
    let sheets: Vec<&[u32]> = pages.chunks(k.per_sheet as usize).collect();
    let idx = k.idx.min(sheets.len() - 1);
    let dir = work_dir(&pdf)?;
    let pngs = sheets[idx]
        .iter()
        .map(|n| {
            let png = dir.join(format!("page-{n}.png")).to_string_lossy().into_owned();
            render_page(&pdf, *n, &png).map(|_| png)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (png, title) = if k.per_sheet == 1 {
        let title = fill(t().preview_page, &[("n", &sheets[idx][0]), ("i", &(idx + 1)), ("count", &sheets.len()), ("scale", &k.scale)]);
        (pngs[0].clone(), title)
    } else {
        // named by its pages, so the terminal is sent the new image when they change
        let png = dir.join(format!("sheet-{}-{}-{}.png", k.paper, k.per_sheet, join(sheets[idx]))).to_string_lossy().into_owned();
        sheet_png(&pngs, k.paper, k.per_sheet, &png)?;
        let title = fill(t().preview_sheet, &[("i", &(idx + 1)), ("count", &sheets.len()), ("pages", &join(sheets[idx])), ("scale", &k.scale)]);
        (png, title)
    };
    Ok(PrintView { thumb: thumbnail(&png)?, png, idx, count: sheets.len(), title })
}

fn name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Inverse of expand_home, for showing paths.
fn tilde(p: &str) -> String {
    let home = std::env::home_dir().map(|h| h.to_string_lossy().into_owned());
    match home.and_then(|h| p.strip_prefix(&h).map(str::to_string)) {
        Some(rest) if rest.starts_with(['/', '\\']) => format!("~{rest}"),
        _ => p.to_string(),
    }
}

/// `~/x` (or `~\x`) to the full path in the home folder.
fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/").or(p.strip_prefix("~\\")), std::env::home_dir()) {
        // one kind of separator on Windows: ~/Documents/x -> C:\\Users\\me\\Documents\\x
        (Some(rest), Some(home)) => home.join(rest.replace('/', std::path::MAIN_SEPARATOR_STR)).to_string_lossy().into_owned(),
        _ => p.to_string(),
    }
}

fn draw(f: &mut Frame, app: &App) {
    if let (Some(bg), Some(fg)) = (theme().bg, theme().fg) {
        f.render_widget(Block::default().style(Style::new().bg(bg).fg(fg)), f.area());
    }
    let rows = app.rows();
    let [main, help] = Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(f.area());
    let [left, preview] = Layout::horizontal([Constraint::Min(40), Constraint::Percentage(45)]).areas(main);
    let [form, status] =
        Layout::vertical([Constraint::Length(rows as u16 + 2), Constraint::Min(3)]).areas(left);

    let lines: Vec<Line> = (0..rows).map(|i| form_row(app, label(app.tab, i), i)).collect();
    let tab = |name: &'static str, on: bool| {
        Span::styled(format!(" {name} "), if on { Style::new().fg(WHITE).bg(ACCENT).bold() } else { Style::new().fg(theme().dim) })
    };
    // same widths as the mouse expects: " PrinterTUI  " then " Print " and " Scan "
    let title = Line::from(vec![Span::styled(" PrinterTUI  ", Style::new().bold()), tab(t().tab_print, app.tab == Tab::Print), tab(t().tab_scan, app.tab == Tab::Scan), tab(t().tab_settings, app.tab == Tab::Settings), Span::raw(" ")]);
    f.render_widget(Paragraph::new(lines).block(panel(title)), form);
    app.form_area.set(form);
    draw_preview(f, app, preview);
    draw_stage(f, app, status);

    let t = t();
    let keys: &[(&str, &str)] = match app.mode {
        Mode::Insert => &[("Esc", t.k_done), ("Enter", t.k_done)],
        Mode::Address(_) => &[("Enter", t.k_add), ("Esc", t.k_cancel)],
        _ if app.tab == Tab::Settings && app.sel == FOLDER => &[("j/k", t.k_move), ("i", t.k_edit_text), ("Tab", t.k_print_scan), ("q", t.k_quit)],
        _ if app.tab == Tab::Settings => &[("j/k", t.k_move), ("h/l", t.k_change), ("Tab", t.k_print_scan), ("q", t.k_quit)],
        _ if app.sel == PAGE && app.tab == Tab::Scan => &[("H/L", t.k_page), ("</>", t.k_move), ("r/R", t.k_rotate), ("f", t.k_filter), ("x", t.k_keep), ("dd", t.k_delete)],
        _ if app.tab == Tab::Scan && !app.scans.is_empty() => &[("j/k", t.k_move), ("h/l", t.k_change), ("H/L", t.k_page), ("Enter", t.k_select), ("Tab", t.k_print_scan), ("q", t.k_quit)],
        _ if app.sel == PAGES && app.tab == Tab::Print => &[("j/k", t.k_move), ("Enter", t.k_pick_pages), ("i", t.k_type_range), ("q", t.k_quit)],
        _ if app.tab == Tab::Print => &[("j/k", t.k_move), ("h/l", t.k_change), ("H/L", t.k_preview_page), ("i", t.k_edit_text), ("Enter", t.k_select), ("Tab", t.k_print_scan), ("q", t.k_quit)],
        _ => &[("j/k", t.k_move), ("h/l", t.k_change), ("i", t.k_edit_text), ("Enter", t.k_select), ("Tab", t.k_print_scan), ("q", t.k_quit)],
    };
    let chips: Vec<Span> = keys
        .iter()
        .flat_map(|(k, what)| [Span::styled(format!(" {k} "), Style::new().fg(theme().chip.0).bg(theme().chip.1)), Span::styled(format!(" {what}  "), Style::new().fg(theme().dim))])
        .collect();
    f.render_widget(Line::from(chips), help);
    let tick = app.started.elapsed().as_millis() as usize;

    match &app.mode {
        Mode::Main | Mode::Insert => {}
        Mode::Address(addr) => {
            let area = popup(f, 76, 4);
            let text = vec![Line::from(format!(" {}: {addr}_", t.address)), Line::from(format!(" {}", t.address_eg)).style(Style::new().add_modifier(Modifier::DIM))];
            f.render_widget(Paragraph::new(text).block(panel(format!(" {} ", t.add_by_address)).border_style(Style::new().fg(ACCENT))), area);
        }
        Mode::Pick(found, state) => {
            let area = popup(f, 76, found.len() as u16 + 2);
            let items: Vec<ListItem> = found.iter().map(|(n, u)| ListItem::new(format!("{n}  {u}"))).collect();
            let list = List::new(items)
                .block(panel(format!(" {} ", t.add_title)).border_style(Style::new().fg(ACCENT)))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, &mut state.clone());
        }
        Mode::Pages(on, state) => {
            let area = popup(f, 76, on.len() as u16 + 2);
            let items: Vec<ListItem> =
                on.iter().enumerate().map(|(i, b)| ListItem::new(format!(" [{}] {}", if *b { "x" } else { " " }, fill(t.page_item, &[("n", &(i + 1))])))).collect();
            let n = on.iter().filter(|b| **b).count();
            let list = List::new(items)
                .block(panel(format!(" {} ", fill(t.pages_title, &[("n", &n), ("all", &on.len())]))).border_style(Style::new().fg(ACCENT)))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, area, &mut state.clone());
        }
        Mode::Queue(jobs, state) => {
            let area = popup(f, 90, jobs.len().max(1) as u16 + 2);
            let items: Vec<ListItem> = if jobs.is_empty() {
                vec![ListItem::new(format!(" {}", t.no_jobs))]
            } else {
                jobs.iter().map(|(id, desc)| ListItem::new(format!(" #{id}  {desc}"))).collect()
            };
            let list = List::new(items)
                .block(panel(format!(" {} ", t.queue_title)).border_style(Style::new().fg(ACCENT)).title_bottom(format!(" {} ", app.status)))
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
            let block = panel(format!(" {} ", t.duplex_title)).border_style(Style::new().fg(ACCENT));
            let [text, anim] = Layout::horizontal([Constraint::Min(30), Constraint::Length(31)]).areas(block.inner(area));
            f.render_widget(block, area);
            f.render_widget(Paragraph::new(app.status.as_str()).wrap(Wrap { trim: false }), text);
            if job.is_none() {
                let (art, caption) = pet::flip(tick / 350);
                let mut lines: Vec<Line> = art.into_iter().map(|l| l.centered()).collect();
                lines.push(Line::default());
                lines.extend(t.flip[caption].lines().map(|c| Line::from(c).centered()));
                f.render_widget(Paragraph::new(lines), anim);
            }
        }
    }
}

/// "Error: ..." in the current language; the printer jams on it.
fn failed(e: impl std::fmt::Display) -> String {
    format!("{}: {e}", t().error)
}

/// Columns a text takes on screen (Chinese characters take two).
fn width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Lines `text` takes wrapped at word boundaries into `cols` columns, as the bubble's paragraph wraps it.
fn wrapped_rows(text: &str, cols: usize) -> usize {
    text.lines()
        .map(|line| {
            let (mut rows, mut x) = (1, 0);
            for word in line.split(' ') {
                let w = width(word);
                if x > 0 && x + 1 + w > cols {
                    (rows, x) = (rows + 1, 0);
                }
                // a word longer than the line is cut into pieces
                rows += w.saturating_sub(1) / cols;
                x = if x == 0 { w % cols.max(1) } else { x + 1 + w };
            }
            rows
        })
        .sum()
}

#[test]
fn bubble_wrapping() {
    assert_eq!(wrapped_rows("request id is Smart_Tank-73 (1 file(s))", 23), 3);
    assert_eq!(wrapped_rows("aaaa bbbb", 9), 1);
    assert_eq!(wrapped_rows("aaaa bbbbb", 9), 2);
    assert_eq!(wrapped_rows("one\ntwo", 20), 2);
    assert_eq!(wrapped_rows(&"x".repeat(25), 10), 3);
}

/// The character at a screen column of `s`, for mouse clicks.
fn char_at(s: &str, col: usize) -> Option<char> {
    let mut x = 0;
    s.chars().find(|c| {
        x += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
        x > col
    })
}

/// A form row's label: Print tab rows past Scale and Scan tab buttons have none.
fn label(tab: Tab, row: usize) -> &'static str {
    match (tab, row) {
        (Tab::Print, 0..=9) => t().labels[row],
        (Tab::Scan, 0..=4) => t().scan_labels[row],
        (Tab::Scan, PAGE) => t().scan_labels[5],
        (Tab::Settings, 0..SETTINGS_ROWS) => t().settings_labels[row],
        _ => "",
    }
}

/// Columns for the labels: the widest in this language, and a space.
fn label_width() -> usize {
    t().labels.iter().chain(&t().scan_labels).chain(&t().settings_labels).map(|l| width(l)).max().unwrap_or(0).max(11) + 1
}

/// The Settings tab's right panel: the settings file as it is on disk, updated as it changes.
fn draw_config(f: &mut Frame, _app: &App, area: ratatui::layout::Rect) {
    let path = config_path().map(|p| tilde(&p.to_string_lossy())).unwrap_or_default();
    let block = panel(format!(" {path} "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let text = config_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let mut lines = vec![Line::styled(fill(t().saved_to, &[("path", &path)]), Style::new().fg(theme().dim)), Line::default()];
    lines.extend(parse_config(&text).into_iter().map(|(k, v)| {
        Line::from(vec![Span::styled(k.to_string(), Style::new().fg(ACCENT)), Span::styled("=", Style::new().fg(theme().dim)), Span::raw(v.to_string())])
    }));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Column of the values in a form row: after the border, " ▶ " and the labels.
fn value_col() -> u16 {
    1 + 3 + label_width() as u16
}

/// A rounded panel with a title in the accent color.
fn panel<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme().dim))
        .title(title.into().style(Style::new().fg(ACCENT).bold()))
}

/// A form row: " ▶ " on the selected one, the label, then the value: `< x >` as ‹ x ›, a
/// `[ button ]` as a pill. Both keep their width, so mouse clicks land on the same characters.
fn form_row<'a>(app: &App, label: &'a str, i: usize) -> Line<'a> {
    let on = i == app.sel;
    let value = app.value(i);
    let marker = if on { Span::styled(" ▶ ", Style::new().fg(ACCENT).bold()) } else { Span::raw("   ") };
    let pad = " ".repeat(label_width().saturating_sub(width(label)));
    let label = Span::styled(format!("{label}{pad}"), if on { Style::new().bold() } else { Style::new().fg(theme().dim) });
    let mut spans = vec![marker, label];
    if let Some(name) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        let style = if on { Style::new().fg(WHITE).bg(ACCENT).bold() } else { Style::new().fg(ACCENT).bold() };
        spans.push(Span::styled(format!(" {name} "), style));
    } else if let Some(inner) = value.strip_prefix("< ").and_then(|v| v.strip_suffix(" >")) {
        let arrows = Style::new().fg(if on { ACCENT } else { theme().dim });
        let hop = on && matches!(app.act, Act::Hop(_)) && !app.act.done();
        let v = if hop { Style::new().fg(theme().hop).bold() } else if on { Style::new().bold() } else { Style::new() };
        spans.extend([Span::styled("‹ ", arrows), Span::styled(inner.to_string(), v), Span::styled(" ›", arrows)]);
    } else {
        let cursor = if on && matches!(app.mode, Mode::Insert) { "_" } else { "" };
        spans.push(Span::styled(format!("{value}{cursor}"), if on { Style::new().bold() } else { Style::new() }));
    }
    Line::from(spans)
}

/// The printer and what it says (the status, or a word about the selected row), and the ink tanks.
fn draw_stage(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let inks = if app.tab != Tab::Print { &[][..] } else { app.printer_state().map_or(&[][..], |s| s.ink.as_slice()) };
    // the printer needs 22 columns and goes last on a narrow screen, after the ink tanks
    let pet_w = if app.mascot && area.width >= 22 + 24 { 22 } else { 0 };
    let tanks_w = if inks.is_empty() || area.width < pet_w + 24 + inks.len() as u16 * 4 + 1 { 0 } else { inks.len() as u16 * 4 + 1 };
    let [pet, bubble, tanks] =
        Layout::horizontal([Constraint::Length(pet_w), Constraint::Min(10), Constraint::Length(tanks_w)]).areas(area);
    let tick = app.started.elapsed().as_millis();
    let (text, work) = match &app.busy {
        Some((msg, _)) => (format!("{msg}..."), if app.scanning { Work::Scan } else { Work::Busy }),
        None if app.editing.is_some() => (format!("{}...", t().editing_page), Work::Busy),
        None if app.viewing.is_some() && app.tab == Tab::Print && app.status.is_empty() => (format!("{}...", t().drawing_preview), Work::Busy),
        None if matches!(app.mode, Mode::Flip { .. }) => (t().flip_time.into(), Work::None),
        None if app.status.is_empty() => (app.complaint().unwrap_or_else(|| app.chat()), Work::None),
        None => (app.status.clone(), Work::None),
    };
    if pet_w > 0 {
        f.render_widget(Paragraph::new(pet::printer(app.act, work, app.mood(), tick)), pet);
    }
    let error = text.starts_with(t().error);
    let worried = !error && app.complaint().is_some_and(|c| c == text);
    let style = if error { Style::new().fg(RED) } else { Style::new() };
    let edge = if error { RED } else if worried { YELLOW } else { ACCENT };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(edge))
        .padding(ratatui::widgets::Padding::horizontal(1));
    // as tall as its text
    let rows = wrapped_rows(&text, bubble.width.saturating_sub(4).max(1) as usize);
    let bubble = ratatui::layout::Rect { height: (rows as u16 + 2).clamp(3, bubble.height), ..bubble };
    f.render_widget(Paragraph::new(text).style(style).wrap(Wrap { trim: false }).block(block), bubble);
    if pet_w > 0 && bubble.height > 3 {
        // the bubble's tail points at the printer
        f.buffer_mut()[(bubble.x, bubble.y + 2)].set_symbol("◀").set_fg(edge);
    }
    if tanks_w > 0 {
        let mut lines = vec![Line::styled(format!(" {}", t().ink), Style::new().fg(theme().dim))];
        lines.extend(pet::tanks(inks).into_iter().map(|l| {
            let mut l = l;
            l.spans.insert(0, Span::raw(" "));
            l
        }));
        f.render_widget(Paragraph::new(lines), tanks);
    }
}

/// The scanned page or the page to print: as an image, or grayscale half-blocks (two pixels per cell).
fn draw_preview(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.tab == Tab::Settings {
        return draw_config(f, app, area);
    }
    let title = match (app.tab == Tab::Scan, app.scans.get(app.cur), &app.view) {
        (true, Some(p), _) => format!(" {}{} ", fill(t().preview_scan, &[("n", &(app.cur + 1)), ("all", &app.scans.len())]), if p.keep { "" } else { t().not_saved }),
        (false, _, Some((_, Ok(v)))) => format!(" {}: {} ", t().preview, v.title),
        _ => format!(" {} ", t().preview),
    };
    let block = panel(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.preview_area.set(inner);
    if app.tab == Tab::Print {
        let note = match &app.view {
            _ if app.viewing.is_some() => Some(t().rendering),
            Some((_, Err(e))) => Some(e.as_str()),
            _ => None,
        };
        if let Some(note) = note {
            f.render_widget(Paragraph::new(note).wrap(Wrap { trim: false }).style(Style::new().add_modifier(Modifier::DIM)), inner);
            return;
        }
    }
    if app.shown().is_none() {
        return;
    }
    match app.graphics {
        Graphics::Kitty(_) => {
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
        Graphics::Sixel => {
            // Sixel images are drawn directly by sync_image() at the preview coordinates.
            // Leaving the inner cells empty in Ratatui's buffer ensures Ratatui diffing
            // never overwrites them on subsequent redraws.
            return;
        }
        Graphics::None => {}
    }
    let Some((_, (w, h, px))) = app.shown() else { return };
    let (cols, rows) = (inner.width as usize, inner.height as usize * 2);
    let scale = (cols as f32 / *w as f32).min(rows as f32 / *h as f32);
    let (ow, oh) = (((*w as f32 * scale) as usize).max(1), ((*h as f32 * scale) as usize).max(2) & !1);
    let small = downscale(*w, *h, px, ow, oh);
    // contrast stretch (30%..97%) so text blocks stand out from the paper
    let gray = |v: u8| {
        let v = ((v as i32 - 77) * 255 / 170).clamp(0, 255) as u8;
        pet::rgb(v, v, v)
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

thread_local! {
    /// The last popup drawn, copied into App::popup_area after each draw.
    static POPUP: Cell<ratatui::layout::Rect> = Cell::default();
}

fn popup(f: &mut Frame, width: u16, height: u16) -> ratatui::layout::Rect {
    let a = f.area();
    let w = a.width.saturating_sub(4).min(width);
    let h = height.min(a.height);
    let area = ratatui::layout::Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h);
    f.render_widget(Clear, area);
    if let (Some(bg), Some(fg)) = (theme().bg, theme().fg) {
        f.render_widget(Block::default().style(Style::new().bg(bg).fg(fg)), area);
    }
    POPUP.set(area);
    area
}
