mod graphics;
mod i18n;
mod pet;
mod print;
mod scan;
mod sound;
mod ui;

use graphics::*;
use i18n::{fill, t};
use ui::*;
use pet::{theme, Act, Mood, Work, ACCENT, RED, WHITE, YELLOW};
use print::*;
use printertui::*;
use sound::Sound;
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

// Print rows
const PRINTER: usize = 0;
const FILE: usize = 1;
const COLOR: usize = 2;
const SIDES: usize = 3;
const BACK_ORDER: usize = 4;
const PAGES: usize = 5;
const PAPER: usize = 6;
const COPIES: usize = 7;
const PER_SHEET_ROW: usize = 8;
const SCALE: usize = 9;
const PRINT: usize = 10;
const ADD: usize = 11;
const QUEUE: usize = 12;

// Scan rows
const SCANNER: usize = 0;
const MODE: usize = 1;
const RESOLUTION: usize = 2;
const FORMAT: usize = 3;
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

// one Mode at a time, so the big duplex variant costs nothing
#[allow(clippy::large_enum_variant)]
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
    /// instructions shown after it, `back` the back-side job, `files[next..]` the files still to
    /// print with `print`'s settings.
    Flip { job: Option<String>, steps: String, back: Job, files: Vec<String>, next: usize, print: PrintSettings },
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
    /// Slow work (scanning, converting, printing, discovery) runs in a thread so the UI keeps drawing.
    busy: Option<(String, mpsc::Receiver<Done>)>,
    /// The busy work is sending a print, which quitting waits for.
    sending: bool,
    /// Quit once the print being sent is in the printer's queue.
    quit_after: bool,
    /// The print queue / duplex front job check running in the background.
    polling: Option<mpsc::Receiver<Done>>,
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = args.iter().find(|a| !a.starts_with("--")).cloned().unwrap_or_default();
    let tab = if args.iter().any(|a| a == "--scan") { Tab::Scan } else { Tab::Print };
    let mut app = App::new(file, tab);
    app.graphics = detect_graphics();
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
    // With --scan, the last page a session that did not quit (it crashed, or was killed) scanned
    // comes back, before the temp folder is tidied
    if app.tab == Tab::Scan
        && let Some(path) = last_scan()
        && let Ok(thumb) = thumbnail(&path)
    {
        app.scans.push(ScannedPage { orig: path.clone(), file: path, rot: 0, filter: 0, keep: true, thumb });
        app.status = t().loaded_last.into();
    }
    std::thread::spawn(clean_temp);

    let mut term = init();
    let res = run(&mut term, &mut app);
    if let (Graphics::Kitty(tmux), Some(_)) = (app.graphics, &app.sent) {
        emit(&kitty(&format!("a=d,d=I,i={},q=2", image_id()), "", tmux));
    }
    restore();
    stop_all();
    res
}

/// The newest scanned page left in any session's scan folder.
fn last_scan() -> Option<String> {
    let sessions = std::fs::read_dir(scan_dir().parent()?).ok()?;
    sessions
        .flatten()
        .filter_map(|s| std::fs::read_dir(s.path()).ok())
        .flatten()
        .flatten()
        // scans are page-<milliseconds>, with no extension; edited copies and previews are .png
        .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("page-")?.parse::<u128>().ok().map(|ms| (ms, e.path())))
        .max()
        .map(|(_, p)| p.to_string_lossy().into_owned())
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
            app.sending = false;
            done(app);
            if app.quit_after {
                return Ok(());
            }
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
        if let Some(done) = app.polling.as_ref().and_then(finished) {
            app.polling = None;
            done(app);
        }
        app.check_printer();
        app.sync_view();
        app.poll_jobs();
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
            match app.on_key(k, &mut pending) {
                Step::Stay => {}
                Step::Quit => return Ok(()),
                Step::PickFiles => {
                    restore();
                    let picked = pick_files();
                    *term = init();
                    app.picked(picked);
                }
                Step::Add(name, uri) => app.add(term, &name, &uri),
            }
        }
    }
}

/// What the run loop does after a key.
#[derive(Debug, PartialEq)]
enum Step {
    Stay,
    Quit,
    /// Open the file picker.
    PickFiles,
    /// Add the printer (name, address).
    Add(String, String),
}

/// Keeps a list's selection on one of its `len` items.
fn clamp(state: &mut ListState, len: usize) {
    if let Some(i) = state.selected() {
        state.select(Some(i.min(len.saturating_sub(1))));
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
    /// The app as it starts, before the printers and settings are read.
    fn new(file: String, tab: Tab) -> App {
        App {
            printers: Vec::new(),
            labels: Vec::new(),
            state: None,
            checking: None,
            checked: Instant::now(),
            printer: 0,
            file,
            color: false,
            duplex: false,
            reverse_back: false,
            pages: String::new(),
            paper: 0,
            copies: 1,
            per_sheet: 0,
            scale: SCALES.iter().position(|s| *s == 100).unwrap_or(0),
            tab,
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
            sending: false,
            quit_after: false,
            polling: None,
            scanning: false,
            clear: false,
            lang: String::new(),
            editing: None,
            graphics: Graphics::None,
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
        }
    }

    /// What a key does; what needs the terminal in normal mode (the file picker, adding a
    /// printer, which may ask for a password) is left to the caller as the returned Step.
    fn on_key(&mut self, k: KeyEvent, pending: &mut Option<char>) -> Step {
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return Step::Quit;
        }
        // one thing at a time
        if self.busy.is_some() && k.code == KeyCode::Enter {
            return Step::Stay;
        }
        // vim `gg`: a second `g` right after the first
        let prev = pending.take();
        let (gg, dd) = (prev == Some('g') && k.code == KeyCode::Char('g'), prev == Some('d') && k.code == KeyCode::Char('d'));
        if let KeyCode::Char(c @ ('g' | 'd')) = k.code
            && prev != Some(c)
            && !matches!(self.mode, Mode::Insert)
        {
            *pending = Some(c);
        }
        match &mut self.mode {
            Mode::Main => match k.code {
                // H/L browse the scanned pages from any row
                KeyCode::Char(c @ ('H' | 'L')) if self.tab == Tab::Scan && !self.scans.is_empty() => {
                    let n = self.scans.len();
                    self.cur = (self.cur + if c == 'L' { 1 } else { n - 1 }) % n;
                }
                // H/L browse the preview pages on the Print tab
                KeyCode::Char(c @ ('H' | 'L')) if self.tab == Tab::Print => {
                    if let Some((_, Ok(v))) = &self.view {
                        self.view_page = (v.idx + if c == 'L' { 1 } else { v.count - 1 }) % v.count;
                    }
                }
                KeyCode::Char(c) if self.tab == Tab::Scan && self.sel == PAGE && !self.scans.is_empty() && self.page_key(c, dd) => {}
                // a print still being sent would be lost: wait for it, unless asked twice
                KeyCode::Esc | KeyCode::Char('q') if self.sending && !self.quit_after => {
                    self.quit_after = true;
                    if let Some((msg, _)) = &mut self.busy {
                        *msg = t().sending_first.into();
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => return Step::Quit,
                KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.checked_sub(1).unwrap_or(self.last()),
                KeyCode::Down | KeyCode::Char('j') => self.sel = (self.sel + 1) % (self.last() + 1),
                KeyCode::Char('g') if gg => self.sel = 0,
                KeyCode::Char('G') => self.sel = self.last(),
                KeyCode::Tab | KeyCode::BackTab => {
                    let i = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
                    self.tab = TABS[(i + if k.code == KeyCode::Tab { 1 } else { TABS.len() - 1 }) % TABS.len()];
                    self.sent = None;
                    self.sel = 0;
                    self.status = String::new();
                    if self.tab == Tab::Scan && self.scanners.is_none() {
                        self.find_scanners(false);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => self.cycle(false),
                KeyCode::Right | KeyCode::Char('l') => self.cycle(true),
                KeyCode::Char('i') | KeyCode::Char('a') if self.text().is_some() => self.mode = Mode::Insert,
                KeyCode::Enter if self.tab == Tab::Scan => self.scan_enter(),
                KeyCode::Enter if self.tab == Tab::Settings && self.sel == FOLDER => self.mode = Mode::Insert,
                KeyCode::Enter if self.tab == Tab::Settings => self.cycle(true),
                KeyCode::Enter if self.sel == ADD => self.find_printers(),
                KeyCode::Enter if self.sel == FILE => return Step::PickFiles,
                KeyCode::Enter if self.sel == PAGES => self.open_pages(),
                KeyCode::Enter if self.sel == QUEUE => {
                    self.status = String::new();
                    self.mode = Mode::Queue(queue(), ListState::default().with_selected(Some(0)));
                }
                KeyCode::Enter => self.try_print(),
                _ => {}
            },
            Mode::Insert => match k.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.mode = Mode::Main;
                    if self.tab == Tab::Settings {
                        self.save_as = default_scan_name(&self.scan_folder);
                        self.save_settings();
                    }
                }
                KeyCode::Backspace => {
                    self.text().map(String::pop);
                }
                KeyCode::Char(c) => {
                    if let Some(t) = self.text() {
                        t.push(c)
                    }
                }
                _ => {}
            },
            Mode::Pick(found, state) => match k.code {
                KeyCode::Esc => self.mode = Mode::Main,
                KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                KeyCode::Char('g') if gg => state.select_first(),
                KeyCode::Char('G') => state.select_last(),
                KeyCode::Char('a') => self.mode = Mode::Address(String::new()),
                KeyCode::Enter => {
                    let (name, uri) = found[state.selected().unwrap_or(0).min(found.len() - 1)].clone();
                    return Step::Add(name, uri);
                }
                _ => {}
            },
            Mode::Address(addr) => match k.code {
                KeyCode::Esc => self.mode = Mode::Main,
                KeyCode::Backspace => {
                    addr.pop();
                }
                KeyCode::Char(c) => addr.push(c),
                KeyCode::Enter if !addr.trim().is_empty() => {
                    let addr = addr.trim().to_string();
                    // a bare IP or name gets the standard IPP Everywhere path; a full uri is used as is
                    let uri = if addr.contains("://") { addr.clone() } else { format!("ipp://{addr}/ipp/print") };
                    let host = uri_host(&uri).map_or(addr.clone(), |(_, h)| h.to_string());
                    return Step::Add(queue_name(&host), uri);
                }
                _ => {}
            },
            Mode::Pages(on, state) => {
                let i = state.selected().unwrap_or(0);
                match k.code {
                    KeyCode::Esc => self.mode = Mode::Main,
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
                            self.status = t().select_a_page.into();
                        } else {
                            self.pages = if picked.len() == on.len() { String::new() } else { join(&picked) };
                            self.mode = Mode::Main;
                        }
                    }
                    _ => {}
                }
            }
            Mode::Queue(jobs, state) => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Main,
                KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                KeyCode::Char('g') if gg => state.select_first(),
                KeyCode::Char('G') => state.select_last(),
                KeyCode::Char('x') | KeyCode::Delete => {
                    if let Some((id, desc)) = state.selected().and_then(|i| jobs.get(i)).cloned() {
                        self.status = match cancel_job(&id) {
                            Ok(()) => fill(t().cancelled, &[("desc", &desc)]),
                            Err(e) => failed(e),
                        };
                        // the list catches up at the next check, which is due right away
                        jobs.retain(|(j, _)| *j != id);
                        self.last_poll = self.last_poll.checked_sub(Duration::from_secs(1)).unwrap_or(self.last_poll);
                    }
                }
                _ => {}
            },
            Mode::Flip { job, .. } => match k.code {
                KeyCode::Esc => {
                    self.status = t().back_cancelled.into();
                    self.mode = Mode::Main;
                }
                KeyCode::Enter if job.is_none() => self.back_side(),
                _ => {}
            },
        }
        // the lists move past their ends (and to usize::MAX for G): back onto their last item
        if let Mode::Pick(list, state) | Mode::Queue(list, state) = &mut self.mode {
            clamp(state, list.len());
        }
        if let Mode::Pages(on, state) = &mut self.mode {
            clamp(state, on.len());
        }
        Step::Stay
    }

    /// Puts the files picked in the dialog or file manager into the File row.
    fn picked(&mut self, picked: Vec<String>) {
        if picked.is_empty() {
            self.status = fill(t().no_file_picked, &[("hint", &t().hints()[0])]);
        } else {
            self.file = picked.join("; ");
        }
    }

    /// Looks for printers on the network to add, then shows them (or asks for an address).
    fn find_printers(&mut self) {
        self.spawn(t().searching_printers, || {
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
        });
    }

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
            Err(e) => failed(e),
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
                SCANNER => self.scanner = step(self.scanner, self.scanners.as_ref().map_or(0, Vec::len)),
                MODE => self.scan_mode = step(self.scan_mode, SCAN_MODES.len()),
                RESOLUTION => self.scan_dpi = step(self.scan_dpi, SCAN_DPI.len()),
                FORMAT => self.scan_format = step(self.scan_format, SCAN_FORMATS.len()),
                PAGE => self.cur = step(self.cur, self.scans.len()),
                _ => {}
            }
            return;
        }
        match self.sel {
            PRINTER => self.printer = step(self.printer, self.printers.len()),
            COLOR => self.color = !self.color,
            SIDES => self.duplex = !self.duplex,
            BACK_ORDER => self.reverse_back = !self.reverse_back,
            PAPER => self.paper = step(self.paper, PAPERS.len()),
            COPIES => self.copies = if fwd { self.copies + 1 } else { (self.copies - 1).max(1) },
            PER_SHEET_ROW => self.per_sheet = step(self.per_sheet, PER_SHEET.len()),
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
                SCANNER => pick(self.scanners.as_ref().and_then(|l| l.get(self.scanner)).map_or(t().none, |(_, d)| d.as_str())),
                MODE => pick(t().scan_modes[self.scan_mode]),
                RESOLUTION => pick(&format!("{} dpi", SCAN_DPI[self.scan_dpi])),
                FORMAT => pick(match SCAN_FORMATS[self.scan_format] {
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
            PRINTER => pick(self.labels.get(self.printer).map_or(t().none, String::as_str)),
            FILE if matches!(self.mode, Mode::Insert) => self.file.clone(),
            FILE => match split_files(&self.file)[..] {
                [] => String::new(),
                [ref one] => one.clone(),
                ref many => {
                    let names: Vec<&str> = many.iter().map(|f| name(f)).collect();
                    fill(t().files, &[("n", &many.len()), ("names", &names.join(", "))])
                }
            },
            COLOR => pick(if self.color { t().color } else { t().grayscale }),
            SIDES => pick(if self.duplex { t().double_sided } else { t().single_sided }),
            BACK_ORDER => pick(if self.reverse_back { t().reversed } else { t().normal }),
            PAGES if self.pages.is_empty() && !matches!(self.mode, Mode::Insert) => t().all.into(),
            PAGES => self.pages.clone(),
            PAPER => pick(PAPERS[self.paper]),
            COPIES => pick(&self.copies.to_string()),
            PER_SHEET_ROW => pick(&PER_SHEET[self.per_sheet].to_string()),
            SCALE => pick(&format!("{}%", SCALES[self.scale])),
            PRINT => format!("[ {} ]", t().btn_print),
            ADD => format!("[ {} ]", t().btn_add),
            _ => format!("[ {} ]", t().btn_queue),
        }
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

}

impl App {
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
                SCANNER if self.scanners.as_ref().is_some_and(Vec::is_empty) => t().no_scanner_yet.into(),
                SCANNER => t().search_again.into(),
                MODE => fill(t().mode_chat, &[("mode", &t().scan_modes[self.scan_mode])]),
                RESOLUTION if SCAN_DPI[self.scan_dpi] >= 600 => n(t().dpi_high, &SCAN_DPI[self.scan_dpi]),
                RESOLUTION => n(t().dpi_chat, &SCAN_DPI[self.scan_dpi]),
                FORMAT if SCAN_FORMATS[self.scan_format] == "OCR" => t().ocr_chat.into(),
                FORMAT => t().format_chat.into(),
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
            PRINTER if self.printers.is_empty() => t().no_printer_yet.into(),
            PRINTER if self.printer_state().is_some_and(|s| s.ink.iter().any(|i| i.2)) => t().ink_low.into(),
            PRINTER => fill(t().at_service, &[("name", &self.labels.get(self.printer).map_or(t().your_printer, String::as_str))]),
            FILE if self.file.is_empty() => t().feed_me.into(),
            FILE => t().other_files.into(),
            COLOR if self.color => t().colors.into(),
            COLOR => t().grayscale_chat.into(),
            SIDES if self.duplex => t().duplex_chat.into(),
            SIDES => t().one_side.into(),
            BACK_ORDER => t().back_order_chat.into(),
            PAGES => t().pages_chat.into(),
            PAPER => fill(t().paper_chat, &[("paper", &PAPERS[self.paper])]),
            COPIES if self.copies >= 10 => n(t().copies_lots, &self.copies),
            COPIES if self.copies == 1 => t().copies_one.into(),
            COPIES => n(t().copies_many, &self.copies),
            PER_SHEET_ROW if PER_SHEET[self.per_sheet] > 1 => n(t().per_sheet_many, &PER_SHEET[self.per_sheet]),
            PER_SHEET_ROW => t().per_sheet_one.into(),
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

}

/// Where scans go unless the settings say otherwise: the Documents folder wherever the system
/// keeps it (OneDrive on many Windows PCs, ~/Documentos on a Spanish Linux desktop), else home.
fn default_scan_folder() -> String {
    let folder = documents_dir().filter(|d| d.is_dir()).or_else(std::env::home_dir);
    folder.map_or("~".into(), |d| tilde(&d.to_string_lossy()))
}

/// `~/Documents/scan-2026-09-26_154200`, in the folder from the settings when there is one.
fn default_scan_name(folder: &str) -> String {
    let folder = if folder.trim().is_empty() { default_scan_folder() } else { folder.trim().trim_end_matches(['/', '\\']).to_string() };
    format!("{folder}/scan-{}", timestamp())
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

/// "Error: ..." in the current language; the printer jams on it.
fn failed(e: impl std::fmt::Display) -> String {
    format!("{}: {e}", t().error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    /// The app with two printers, and no scanner search when the Scan tab opens.
    fn app() -> App {
        let mut app = App::new(String::new(), Tab::Print);
        app.printers = vec!["Office".into(), "Home".into()];
        app.labels = app.printers.clone();
        app.scanners = Some(Vec::new());
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Presses each key in turn ('\n' is Enter, '\x1b' Esc); returns what the last one asked for.
    fn press(app: &mut App, keys: &str) -> Step {
        let mut pending = None;
        let mut step = Step::Stay;
        for c in keys.chars() {
            let code = match c {
                '\n' => KeyCode::Enter,
                '\x1b' => KeyCode::Esc,
                '\t' => KeyCode::Tab,
                '\x08' => KeyCode::Backspace,
                c => KeyCode::Char(c),
            };
            step = app.on_key(key(code), &mut pending);
        }
        step
    }

    #[test]
    fn moving_and_changing_values() {
        let mut app = app();
        press(&mut app, "j");
        assert_eq!(app.sel, FILE);
        press(&mut app, "kk");
        assert_eq!(app.sel, QUEUE);
        press(&mut app, "gg");
        assert_eq!(app.sel, PRINTER);
        press(&mut app, "l");
        assert_eq!(app.printer, 1);
        press(&mut app, "l");
        assert_eq!(app.printer, 0);
        app.sel = COPIES;
        press(&mut app, "ll");
        assert_eq!(app.copies, 3);
        press(&mut app, "hhhh");
        assert_eq!(app.copies, 1);
        app.sel = COLOR;
        press(&mut app, "l");
        assert!(app.color);
        press(&mut app, "G");
        assert_eq!(app.sel, QUEUE);
    }

    #[test]
    fn typing_a_file_name() {
        let mut app = app();
        app.sel = FILE;
        press(&mut app, "ia b.pdf\x08\x1b");
        assert_eq!(app.file, "a b.pd");
        assert!(matches!(app.mode, Mode::Main));
        // q in the field is a letter, not quitting
        assert_eq!(press(&mut app, "iq"), Step::Stay);
        assert_eq!(app.file, "a b.pdq");
    }

    #[test]
    fn picking_pages_past_the_end_of_the_list() {
        let mut app = app();
        app.mode = Mode::Pages(vec![true; 5], ListState::default().with_selected(Some(0)));
        // all off, the second on, then G (as far as the list goes) and the last one on
        press(&mut app, "ajxGx");
        press(&mut app, "\n");
        assert_eq!(app.pages, "2,5");
        assert!(matches!(app.mode, Mode::Main));
    }

    #[test]
    fn adding_the_printer_picked() {
        let mut app = app();
        let found = vec![("A".to_string(), "ipp://a/ipp/print".to_string()), ("B".to_string(), "ipp://b/ipp/print".to_string())];
        app.mode = Mode::Pick(found, ListState::default().with_selected(Some(0)));
        assert_eq!(press(&mut app, "jjj\n"), Step::Add("B".into(), "ipp://b/ipp/print".into()));
        app.mode = Mode::Address(String::new());
        assert_eq!(press(&mut app, "192.168.1.46\n"), Step::Add("printer_192_168_1_46".into(), "ipp://192.168.1.46/ipp/print".into()));
    }

    #[test]
    fn quitting_waits_for_a_print_being_sent() {
        let mut app = app();
        let (_tx, rx) = mpsc::channel();
        app.busy = Some(("Sending".into(), rx));
        app.sending = true;
        assert_eq!(press(&mut app, "q"), Step::Stay);
        assert!(app.quit_after);
        assert_eq!(press(&mut app, "q"), Step::Quit);
        assert_eq!(app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), &mut None), Step::Quit);
        assert_eq!(press(&mut App::new(String::new(), Tab::Print), "q"), Step::Quit);
    }

    #[test]
    fn tabs_go_round() {
        let mut app = app();
        app.sel = COPIES;
        press(&mut app, "\t");
        assert!(app.tab == Tab::Scan && app.sel == 0);
        press(&mut app, "\t");
        assert!(app.tab == Tab::Settings);
        app.on_key(key(KeyCode::BackTab), &mut None);
        assert!(app.tab == Tab::Scan);
    }

    #[test]
    fn scanned_page_keys() {
        let mut app = app();
        app.tab = Tab::Scan;
        app.sel = PAGE;
        let page = |n: &str| ScannedPage { orig: n.into(), file: n.into(), rot: 0, filter: 0, keep: true, thumb: (1, 1, vec![0]) };
        app.scans = vec![page("/nowhere/page-1"), page("/nowhere/page-2"), page("/nowhere/page-3")];
        press(&mut app, "x");
        assert!(!app.scans[0].keep);
        press(&mut app, ">");
        assert_eq!((app.cur, app.scans[1].orig.as_str()), (1, "/nowhere/page-1"));
        press(&mut app, "dd");
        let left: Vec<&str> = app.scans.iter().map(|p| p.orig.as_str()).collect();
        assert_eq!(left, ["/nowhere/page-2", "/nowhere/page-3"]);
    }

    #[test]
    fn the_screen_and_mouse_clicks() {
        let mut app = app();
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
        let screen: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(screen.contains("PrinterTUI") && screen.contains("Office"), "{screen}");
        // a click on the Copies row selects it, a click on its > adds a copy
        let form = app.form_area.get();
        let click = |x: u16, y: u16| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE };
        let row = form.y + 1 + COPIES as u16;
        assert!(app.mouse(click(form.x + 5, row)).is_empty());
        assert_eq!(app.sel, COPIES);
        // the value reads "< 1 >": its > is 4 columns in
        for k in app.mouse(click(form.x + value_col() + 4, row)) {
            app.on_key(k, &mut None);
        }
        assert_eq!(app.copies, 2);
    }
}
