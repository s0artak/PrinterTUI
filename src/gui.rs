use crate::i18n::{extra, fill, t};
use crate::pet::{Act, Mood, Work};
use crate::sound::Sound;
use crate::{i18n, pet, sound};
use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, Frame, Key, Layout, Margin, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use eframe::{egui_wgpu, wgpu};
use printertui::*;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const RECENT: usize = 5;
const DUMMY_QUEUE: &str = "printertui-demo";
const DUMMY_SCANNER: &str = "dummy";

// Print row indices (for contextual mascot chat)
const R_PRINTER: usize = 0;
const R_FILE: usize = 1;
const R_COLOR: usize = 2;
const R_SIDES: usize = 3;
const R_ORDER: usize = 4;
const R_PAGES: usize = 5;
const R_PAPER: usize = 6;
const R_COPIES: usize = 7;
const R_PER_SHEET: usize = 8;
const R_SCALE: usize = 9;
const R_QUALITY: usize = 10;
const R_PRINT: usize = 11;
const R_ADD: usize = 12;
const R_QUEUE: usize = 13;

// Scan row indices
const S_SCANNER: usize = 0;
const S_SOURCE: usize = 1;
const S_MODE: usize = 2;
const S_DPI: usize = 3;
const S_FORMAT: usize = 4;
const S_SAVE_AS: usize = 5;
const S_SCAN: usize = 6;
const S_PAGE: usize = 7;
const S_SAVE: usize = 8;
const S_COPY: usize = 9;
const S_DISCARD: usize = 10;

// Settings row indices
const C_LANG: usize = 0;
const C_THEME: usize = 1;
const C_VOLUME: usize = 2;
const C_MASCOT: usize = 3;
const C_FOLDER: usize = 5;

type Done = Box<dyn FnOnce(&mut GuiApp) + Send>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Print,
    Scan,
    Settings,
}

#[derive(Clone, Copy)]
struct Palette {
    dark: bool,
    bg: Color32,
    panel: Color32,
    panel_alt: Color32,
    control: Color32,
    control_hover: Color32,
    control_active: Color32,
    input: Color32,
    border: Color32,
    border_strong: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    accent_hover: Color32,
    accent_soft: Color32,
    yellow: Color32,
    red: Color32,
    green: Color32,
    chip_bg: Color32,
    chip_fg: Color32,
}

impl Palette {
    fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(27, 28, 32),
            panel: Color32::from_rgb(34, 36, 42),
            panel_alt: Color32::from_rgb(24, 26, 31),
            control: Color32::from_rgb(43, 46, 54),
            control_hover: Color32::from_rgb(54, 58, 69),
            control_active: Color32::from_rgb(56, 72, 114),
            input: Color32::from_rgb(22, 24, 29),
            border: Color32::from_rgb(58, 62, 74),
            border_strong: Color32::from_rgb(78, 84, 101),
            text: Color32::from_rgb(232, 234, 240),
            muted: Color32::from_rgb(144, 150, 166),
            accent: Color32::from_rgb(95, 135, 255),
            accent_hover: Color32::from_rgb(115, 151, 255),
            accent_soft: Color32::from_rgba_unmultiplied(95, 135, 255, 34),
            yellow: Color32::from_rgb(255, 215, 95),
            red: Color32::from_rgb(255, 95, 95),
            green: Color32::from_rgb(135, 215, 135),
            chip_bg: Color32::from_rgb(45, 48, 58),
            chip_fg: Color32::from_rgb(220, 224, 234),
        }
    }

    fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::from_rgb(242, 244, 248),
            panel: Color32::from_rgb(255, 255, 255),
            panel_alt: Color32::from_rgb(233, 236, 243),
            control: Color32::from_rgb(238, 241, 247),
            control_hover: Color32::from_rgb(225, 230, 240),
            control_active: Color32::from_rgb(212, 224, 255),
            input: Color32::from_rgb(249, 250, 253),
            border: Color32::from_rgb(211, 216, 228),
            border_strong: Color32::from_rgb(180, 189, 208),
            text: Color32::from_rgb(29, 32, 40),
            muted: Color32::from_rgb(95, 102, 119),
            accent: Color32::from_rgb(70, 110, 235),
            accent_hover: Color32::from_rgb(54, 94, 219),
            accent_soft: Color32::from_rgba_unmultiplied(70, 110, 235, 24),
            yellow: Color32::from_rgb(184, 110, 0),
            red: Color32::from_rgb(214, 54, 54),
            green: Color32::from_rgb(42, 143, 74),
            chip_bg: Color32::from_rgb(226, 230, 240),
            chip_fg: Color32::from_rgb(45, 50, 64),
        }
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
    }
    unsafe {
        let _ = AttachConsole(0xFFFF_FFFF);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn print_help() {
    println!(
        "\
PrinterTUI {}

usage: printertui [file...]          open, with these files loaded
       printertui --dummy [file...]   open with a simulated printer & scanner
       printertui --scan             open on the Scan tab
       printertui --print [file...]  print with the saved settings and exit
       printertui --version
       printertui --help

Files are PDFs, photos, text or documents (LibreOffice); several are printed in
order.",
        env!("CARGO_PKG_VERSION")
    );
}

pub fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        attach_parent_console();
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        attach_parent_console();
        println!("printertui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let dummy = args.iter().any(|a| a == "--dummy");
    if args.iter().any(|a| a == "--print") && !dummy {
        attach_parent_console();
        let files: Vec<String> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(|f| expand_home(f))
            .collect();
        let mut app = GuiApp::new_state(false, files.join("; "), false);
        match app.print_now_headless() {
            Ok(out) => {
                println!("{out}");
                return Ok(());
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    let scan = args.iter().any(|a| a == "--scan");
    let file = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    run(scan, file, dummy)
}

fn run(scan: bool, file: String, dummy: bool) -> eframe::Result {
    let (icon_w, icon_h, icon_rgba) = pet::icon_rgba();
    let icon = egui::IconData {
        rgba: icon_rgba,
        width: icon_w,
        height: icon_h,
    };
    let wgpu_setup = egui_wgpu::WgpuSetupCreateNew {
        native_adapter_selector: Some(Arc::new(|adapters, surface| {
            // Prefer discrete/integrated GPUs, but gracefully fall back to CPU (Microsoft WARP)
            // so VMs, RDP sessions, and basic display drivers never fail to open the window.
            let rank = |adapter: &wgpu::Adapter| -> u8 {
                match adapter.get_info().device_type {
                    wgpu::DeviceType::DiscreteGpu => 0,
                    wgpu::DeviceType::IntegratedGpu => 1,
                    wgpu::DeviceType::VirtualGpu => 2,
                    wgpu::DeviceType::Cpu => 3,
                    wgpu::DeviceType::Other => 4,
                }
            };
            adapters
                .iter()
                .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
                .min_by_key(|a| rank(a))
                .cloned()
                .ok_or_else(|| "No suitable GPU or software adapter found".into())
        })),
        ..Default::default()
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PrinterTUI")
            .with_inner_size([1060.0, 720.0])
            .with_min_inner_size([760.0, 580.0])
            .with_icon(Arc::new(icon)),
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(wgpu_setup),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "PrinterTUI",
        options,
        Box::new(move |cc| {
            let mut app = GuiApp::new_state(scan, file, dummy);
            let need_intl = needs_intl_fonts(t().code) || app.tab == Tab::Settings;
            setup_fonts(&cc.egui_ctx, need_intl);
            app.intl_fonts_loaded = need_intl;
            app.apply_theme(&cc.egui_ctx, true);
            // Re-sync native Windows 11 dark titlebar on the first live frame once HWND is visible
            app.applied_dark = None;
            sound::play(Sound::Boot);
            Ok(Box::new(app))
        }),
    )
}

fn needs_intl_fonts(code: &str) -> bool {
    matches!(code, "zh" | "hi" | "bn" | "ar")
}

fn setup_fonts(ctx: &egui::Context, include_intl: bool) {
    let mut fonts = egui::FontDefinitions::default();
    let mut prop_added = Vec::new();
    let mut mono_added = Vec::new();

    if let Some(bytes) = find_font(&["googlesans", "segoeui.ttf", "arial.ttf", "sfns.ttf", "helveticaneue.ttc"]) {
        fonts
            .font_data
            .insert("ui_primary".into(), egui::FontData::from_owned(bytes).into());
        prop_added.push("ui_primary".to_string());
    }
    if let Some(bytes) = find_font(&["consola.ttf", "cascadiamono.ttf", "menlo.ttc", "sfmono"]) {
        fonts
            .font_data
            .insert("mono_primary".into(), egui::FontData::from_owned(bytes).into());
        mono_added.push("mono_primary".to_string());
    }
    if include_intl {
        for (name, candidates) in [
            ("fb_indic", &["nirmala.ttc", "nirmala.ttf"][..]),
            ("fb_cjk", &["msyh.ttc", "yugothr.ttc", "meiryo.ttc", "pingfang.ttc"][..]),
            ("fb_tahoma", &["tahoma.ttf"][..]),
        ] {
            if let Some(bytes) = find_font(candidates) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(bytes).into());
                prop_added.push(name.into());
                mono_added.push(name.into());
            }
        }
    }

    let prop = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    for (i, name) in prop_added.into_iter().enumerate() {
        prop.insert(i, name);
    }
    let mono = fonts.families.entry(egui::FontFamily::Monospace).or_default();
    for (i, name) in mono_added.into_iter().enumerate() {
        mono.insert(i, name);
    }
    ctx.set_fonts(fonts);
}

fn font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        dirs.push(std::path::PathBuf::from(windows).join("Fonts"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(std::path::PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
    }
    dirs.extend([
        std::path::PathBuf::from("/System/Library/Fonts"),
        std::path::PathBuf::from("/System/Library/Fonts/Supplemental"),
        std::path::PathBuf::from("/Library/Fonts"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(home).join("Library/Fonts"));
    }
    dirs
}

fn find_font(patterns: &[&str]) -> Option<Vec<u8>> {
    let dirs = font_dirs();
    for &pat in patterns {
        for dir in &dirs {
            let direct = dir.join(pat);
            if direct.is_file()
                && let Ok(bytes) = std::fs::read(&direct)
            {
                return Some(bytes);
            }
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for entry in rd.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let lower = name.to_ascii_lowercase();
                let ext_ok = lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc");
                if ext_ok && (lower == pat || (pat.find('.').is_none() && lower.starts_with(pat)))
                    && let Ok(bytes) = std::fs::read(&path)
                {
                    return Some(bytes);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn set_titlebar_dark(dark: bool) {
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut std::ffi::c_void,
            attr: u32,
            val: *const i32,
            size: u32,
        ) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> *mut std::ffi::c_void;
        fn GetActiveWindow() -> *mut std::ffi::c_void;
    }
    let title: Vec<u16> = "PrinterTUI\0".encode_utf16().collect();
    unsafe {
        let mut hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            hwnd = GetActiveWindow();
        }
        if !hwnd.is_null() {
            let val = i32::from(dark);
            // DWMWA_USE_IMMERSIVE_DARK_MODE (Windows 10 20H1+ & Windows 11)
            let _ = DwmSetWindowAttribute(hwnd, 20, &val, 4);
        }
    }
}

#[cfg(not(windows))]
fn set_titlebar_dark(_dark: bool) {}

fn apply_visuals(ctx: &egui::Context, p: Palette) {
    set_titlebar_dark(p.dark);
    ctx.set_theme(if p.dark { egui::Theme::Dark } else { egui::Theme::Light });
    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.override_text_color = Some(p.text);
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0_f32, p.border_strong);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(6);
    v.faint_bg_color = p.panel_alt;
    v.extreme_bg_color = p.input;
    v.code_bg_color = p.panel_alt;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.yellow;
    v.error_fg_color = p.red;
    v.selection.bg_fill = p.control_active;
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    v.widgets.noninteractive.bg_fill = p.panel;
    v.widgets.noninteractive.weak_bg_fill = p.panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.muted);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(6);
    for (visual, fill, stroke, fg) in [
        (&mut v.widgets.inactive, p.control, p.border, p.text),
        (&mut v.widgets.hovered, p.control_hover, p.border_strong, p.text),
        (&mut v.widgets.active, p.control_active, p.accent, p.text),
        (&mut v.widgets.open, p.control_hover, p.accent, p.text),
    ] {
        visual.bg_fill = fill;
        visual.weak_bg_fill = fill;
        visual.bg_stroke = Stroke::new(1.0_f32, stroke);
        visual.fg_stroke = Stroke::new(1.0_f32, fg);
        visual.corner_radius = CornerRadius::same(6);
        visual.expansion = 0.0;
    }
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(7.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 4.5);
        s.spacing.interact_size.y = 26.0;
        s.spacing.combo_height = 240.0;
        s.visuals.button_frame = true;
    });
}

struct DummyJob {
    id: String,
    description: String,
    sides: u32,
    started: Instant,
    cancelled: bool,
}

impl DummyJob {
    fn duration(&self) -> Duration {
        Duration::from_millis((2_200 + u64::from(self.sides) * 550).min(12_000))
    }

    fn progress(&self) -> u32 {
        (u128::from(self.sides) * self.started.elapsed().as_millis() / self.duration().as_millis())
            .min(u128::from(self.sides)) as u32
    }

    fn active(&self) -> bool {
        !self.cancelled && self.started.elapsed() < self.duration()
    }
}

#[derive(Clone, PartialEq)]
struct ViewKey {
    file: String,
    pages: String,
    scale: u32,
    paper: &'static str,
    per_sheet: u32,
    idx: usize,
}

struct PrintView {
    idx: usize,
    count: usize,
    title: String,
    image: ColorImage,
    texture: Option<TextureHandle>,
}

struct ScannedPage {
    orig: String,
    file: String,
    rot: u16,
    filter: usize,
    keep: bool,
    image: ColorImage,
    texture: Option<TextureHandle>,
}

enum Modal {
    None,
    Pages(Vec<bool>),
    Queue(Vec<(String, String)>),
    AddPrinter {
        found: Vec<(String, String)>,
        address: String,
    },
    Flip {
        job: Option<String>,
        steps: String,
        back: (Job, usize),
        files: Vec<String>,
        next: usize,
        use_range: bool,
    },
    Doctor(String),
}

struct GuiApp {
    dummy: bool,
    dummy_jobs: Vec<DummyJob>,
    dummy_next_job: u32,
    backend: Option<&'static str>,
    printers: Vec<String>,
    labels: Vec<String>,
    duplex_caps: Vec<bool>,
    state: Option<(String, PrinterState)>,
    checking: Option<mpsc::Receiver<Done>>,
    polling: Option<mpsc::Receiver<Done>>,
    checked: Instant,
    printer: usize,
    file: String,
    recent: Vec<String>,
    color: bool,
    duplex: bool,
    reverse_back: bool,
    reverse: bool,
    pages: String,
    paper: usize,
    copies: u32,
    per_sheet: usize,
    scale: usize,
    quality: usize,
    tab: Tab,
    theme: usize,
    applied_dark: Option<bool>,
    mascot: bool,
    graphics_pref: String,
    scan_folder: String,
    view: Option<(ViewKey, Result<PrintView, String>)>,
    viewing: Option<(ViewKey, mpsc::Receiver<Done>)>,
    view_file: usize,
    view_page: usize,
    scanners: Option<Vec<(String, String)>>,
    scanner: usize,
    scanner_pref: String,
    scan_source: usize,
    scan_mode: usize,
    scan_dpi: usize,
    scan_format: usize,
    save_as: String,
    scans: Vec<ScannedPage>,
    discarded: Vec<ScannedPage>,
    cur: usize,
    busy: Option<(String, mpsc::Receiver<Done>)>,
    scanning: bool,
    editing: Option<mpsc::Receiver<Done>>,
    lang: String,
    started: Instant,
    last_poll: Instant,
    focused_row: usize,
    hop_row: Option<(usize, Instant)>,
    status: String,
    heard: String,
    act: Act,
    printing: Vec<(String, u32, u32)>,
    modal: Modal,
    config_cache: String,
    intl_fonts_loaded: bool,
    text_focused: bool,
}

fn task(work: impl FnOnce() -> Done + Send + 'static) -> mpsc::Receiver<Done> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

fn finished(rx: &mpsc::Receiver<Done>) -> Option<Done> {
    match rx.try_recv() {
        Ok(done) => Some(done),
        Err(mpsc::TryRecvError::Disconnected) => {
            Some(Box::new(|app: &mut GuiApp| app.status = failed(t().task_crashed)))
        }
        Err(mpsc::TryRecvError::Empty) => None,
    }
}

fn decode_color_image(path: &str) -> Result<ColorImage, String> {
    let reader = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| format!("{path}: {e}"))?;
    let mut img = reader.decode().map_err(|e| format!("{path}: {e}"))?;
    if img.width() > 1400 || img.height() > 1800 {
        img = img.resize(1400, 1800, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

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
            let png = dir
                .join(format!("page-{n}.png"))
                .to_string_lossy()
                .into_owned();
            if !fresh(&pdf, std::path::Path::new(&png)) {
                render_page(&pdf, *n, &png)?;
            }
            Ok(png)
        })
        .collect::<Result<Vec<String>, String>>()?;
    let (png, title) = if k.per_sheet == 1 {
        let title = fill(
            t().preview_page,
            &[
                ("n", &sheets[idx][0]),
                ("i", &(idx + 1)),
                ("count", &sheets.len()),
                ("scale", &k.scale),
            ],
        );
        (pngs[0].clone(), title)
    } else {
        let png = dir
            .join(format!(
                "sheet-{}-{}-{}.png",
                k.paper,
                k.per_sheet,
                join(sheets[idx])
            ))
            .to_string_lossy()
            .into_owned();
        if !fresh(&pdf, std::path::Path::new(&png)) {
            sheet_png(&pngs, k.paper, k.per_sheet, &png)?;
        }
        let title = fill(
            t().preview_sheet,
            &[
                ("i", &(idx + 1)),
                ("count", &sheets.len()),
                ("pages", &join(sheets[idx])),
                ("scale", &k.scale),
            ],
        );
        (png, title)
    };
    let image = decode_color_image(&png)?;
    Ok(PrintView {
        idx,
        count: sheets.len(),
        title,
        image,
        texture: None,
    })
}

fn backend_text(problem: Option<&str>) -> Option<String> {
    let cmd = if cfg!(target_os = "macos") {
        "sudo launchctl start org.cups.cupsd"
    } else {
        "sudo systemctl enable --now cups"
    };
    match problem? {
        "cups-missing" => Some(extra().cups_missing.into()),
        "cups-stopped" => Some(fill(extra().cups_stopped, &[("cmd", &cmd)])),
        "spooler-stopped" => Some(extra().spooler_stopped.into()),
        _ => None,
    }
}

fn dummy_state() -> PrinterState {
    PrinterState {
        ink: vec![
            (0x00AEEF, 92, false),
            (0xE83E8C, 78, false),
            (0xFFC107, 66, false),
            (0x222222, 85, false),
        ],
        ink_source: InkSource::Demo,
        problems: Vec::new(),
    }
}

fn dummy_queue(jobs: &[DummyJob]) -> Vec<(String, String)> {
    jobs.iter()
        .filter(|job| job.active())
        .map(|job| {
            (
                job.id.clone(),
                format!(
                    "{}  ·  {} {}/{}",
                    job.description,
                    extra().simulated,
                    job.progress(),
                    job.sides
                ),
            )
        })
        .collect()
}

fn cancel_dummy_job(jobs: &mut [DummyJob], id: &str) -> Result<(), String> {
    let Some(job) = jobs.iter_mut().find(|job| job.id == id && job.active()) else {
        return Err(format!("Unknown demo job {id}"));
    };
    job.cancelled = true;
    Ok(())
}

fn dummy_job_active(jobs: &[DummyJob], id: &str) -> bool {
    jobs.iter()
        .find(|job| job.id == id)
        .is_some_and(DummyJob::active)
}

fn job_no(lp_out: &str) -> String {
    format!(
        "#{}",
        job_id(lp_out)
            .and_then(|id| id.rsplit('-').next())
            .unwrap_or("?")
    )
}

fn failed(e: impl std::fmt::Display) -> String {
    format!("{}: {e}", t().error)
}

fn name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn tilde(p: &str) -> String {
    let home = std::env::home_dir().map(|h| h.to_string_lossy().into_owned());
    match home.and_then(|h| p.strip_prefix(&h).map(str::to_string)) {
        Some(rest) if rest.starts_with(['/', '\\']) => format!("~{rest}"),
        _ => p.to_string(),
    }
}

fn expand_home(p: &str) -> String {
    match (
        p.strip_prefix("~/").or(p.strip_prefix("~\\")),
        std::env::home_dir(),
    ) {
        (Some(rest), Some(home)) => home
            .join(rest.replace('/', std::path::MAIN_SEPARATOR_STR))
            .to_string_lossy()
            .into_owned(),
        _ => p.to_string(),
    }
}

fn default_scan_folder() -> String {
    if let Some(home) = std::env::home_dir()
        && home.join("Documents").is_dir()
    {
        return if cfg!(windows) {
            home.join("Documents").to_string_lossy().into_owned()
        } else {
            "~/Documents".into()
        };
    }
    if cfg!(windows) {
        std::env::home_dir().map_or_else(|| ".".into(), |h| h.to_string_lossy().into_owned())
    } else {
        "~".into()
    }
}

fn default_scan_name(folder: &str) -> String {
    let folder = if folder.trim().is_empty() {
        default_scan_folder()
    } else {
        folder.trim().trim_end_matches(['/', '\\']).to_string()
    };
    let sep = std::path::MAIN_SEPARATOR;
    format!("{folder}{sep}scan-{}", timestamp())
}

fn default_paper() -> usize {
    let locale = system_language();
    let country = locale
        .split(['_', '-'])
        .nth(1)
        .map(|r| r.split('.').next().unwrap_or(r).to_ascii_uppercase())
        .unwrap_or_default();
    let letter = ["US", "CA", "MX", "PH", "CL", "CO", "VE", "BO"].contains(&country.as_str());
    if letter {
        PAPERS.iter().position(|p| *p == "Letter").unwrap_or(0)
    } else {
        0
    }
}

impl GuiApp {
    fn new_state(scan: bool, file: String, dummy: bool) -> Self {
        let mut app = Self {
            dummy,
            dummy_jobs: Vec::new(),
            dummy_next_job: 1,
            backend: if dummy { None } else { backend_problem() },
            printers: Vec::new(),
            labels: Vec::new(),
            duplex_caps: Vec::new(),
            state: None,
            checking: None,
            polling: None,
            checked: Instant::now() - Duration::from_secs(30),
            printer: 0,
            file,
            recent: Vec::new(),
            color: false,
            duplex: false,
            reverse_back: false,
            reverse: false,
            pages: String::new(),
            paper: default_paper(),
            copies: 1,
            per_sheet: 0,
            scale: SCALES.iter().position(|s| *s == 100).unwrap_or(4),
            quality: 1,
            tab: if scan { Tab::Scan } else { Tab::Print },
            theme: 0,
            applied_dark: None,
            mascot: true,
            graphics_pref: "auto".into(),
            scan_folder: String::new(),
            view: None,
            viewing: None,
            view_file: 0,
            view_page: 0,
            scanners: None,
            scanner: 0,
            scanner_pref: String::new(),
            scan_source: 0,
            scan_mode: 0,
            scan_dpi: 1,
            scan_format: 0,
            save_as: String::new(),
            scans: Vec::new(),
            discarded: Vec::new(),
            cur: 0,
            busy: None,
            scanning: false,
            editing: None,
            lang: String::new(),
            started: Instant::now(),
            last_poll: Instant::now(),
            focused_row: 0,
            hop_row: None,
            status: String::new(),
            heard: String::new(),
            act: Act::Idle,
            printing: Vec::new(),
            modal: Modal::None,
            config_cache: String::new(),
            intl_fonts_loaded: false,
            text_focused: false,
        };
        if dummy {
            app.printers = vec![DUMMY_QUEUE.into()];
            app.duplex_caps = vec![false];
            app.state = Some((DUMMY_QUEUE.into(), dummy_state()));
        } else {
            app.set_printers(printers());
        }
        if !cfg!(test)
            && let Some(text) = config_path().and_then(|p| std::fs::read_to_string(p).ok())
        {
            for (k, v) in parse_config(&text) {
                app.apply(k, v);
            }
            app.config_cache = text;
        }
        app.apply_lang_and_demo();
        app.save_as = default_scan_name(&app.scan_folder);
        if dummy {
            app.status = extra().demo_status.into();
        } else if app.printers.is_empty() {
            app.status = backend_text(app.backend).unwrap_or_else(|| t().no_printers.into());
        }
        if app.tab == Tab::Scan && app.scanners.is_none() {
            app.find_scanners(false);
        }
        app
    }

    fn set_printers(&mut self, list: Vec<String>) {
        if self.dummy {
            self.labels = vec![extra().demo_printer.into()];
            self.duplex_caps = vec![false];
        } else {
            self.labels = printer_labels(&list);
            self.duplex_caps = list.iter().map(|q| supports_duplex(q)).collect();
        }
        self.printers = list;
        if self.printer >= self.printers.len() {
            self.printer = 0;
        }
    }

    fn apply(&mut self, key: &str, v: &str) {
        let b = v.parse().ok();
        match key {
            "printer" => {
                self.printer = self
                    .printers
                    .iter()
                    .position(|p| p == v)
                    .unwrap_or(self.printer);
            }
            "color" => self.color = b.unwrap_or(self.color),
            "duplex" => self.duplex = b.unwrap_or(self.duplex),
            "reverse_back" => self.reverse_back = b.unwrap_or(self.reverse_back),
            "reverse" => self.reverse = b.unwrap_or(self.reverse),
            "paper" => {
                self.paper = PAPERS.iter().position(|p| *p == v).unwrap_or(self.paper);
            }
            "copies" => {
                self.copies = v.parse().ok().filter(|n| *n > 0).unwrap_or(self.copies);
            }
            "per_sheet" => {
                self.per_sheet = PER_SHEET
                    .iter()
                    .position(|n| n.to_string() == v)
                    .unwrap_or(self.per_sheet);
            }
            "lang" => self.lang = v.to_string(),
            "theme" => {
                self.theme = pet::THEME_NAMES
                    .iter()
                    .position(|name| *name == v)
                    .unwrap_or(self.theme);
            }
            "volume" => sound::set_volume(v.parse().unwrap_or(sound::volume())),
            "mascot" => self.mascot = b.unwrap_or(self.mascot),
            "graphics" if !v.is_empty() => self.graphics_pref = v.to_string(),
            "scan_folder" => self.scan_folder = v.to_string(),
            "scale" => {
                self.scale = SCALES
                    .iter()
                    .position(|n| n.to_string() == v)
                    .unwrap_or(self.scale);
            }
            "quality" => {
                self.quality = QUALITIES
                    .iter()
                    .position(|q| *q == v)
                    .unwrap_or(self.quality);
            }
            "scanner" => self.scanner_pref = v.to_string(),
            "scan_source" => {
                self.scan_source = SCAN_SOURCES
                    .iter()
                    .position(|s| *s == v)
                    .unwrap_or(self.scan_source);
            }
            "scan_mode" => {
                self.scan_mode = SCAN_MODES
                    .iter()
                    .position(|m| *m == v)
                    .unwrap_or(self.scan_mode);
            }
            "scan_dpi" => {
                self.scan_dpi = SCAN_DPI
                    .iter()
                    .position(|d| d.to_string() == v)
                    .unwrap_or(self.scan_dpi);
            }
            "scan_format" => {
                self.scan_format = SCAN_FORMATS
                    .iter()
                    .position(|f| *f == v)
                    .unwrap_or(self.scan_format);
            }
            "recent" if !self.recent.iter().any(|f| f == v) && self.recent.len() < RECENT => {
                self.recent.push(v.to_string());
            }
            _ => {}
        }
    }

    fn apply_lang_and_demo(&mut self) {
        let code = if self.lang.is_empty() {
            system_language()
        } else {
            self.lang.clone()
        };
        i18n::set(&code);
        if self.dummy {
            self.labels = vec![extra().demo_printer.into()];
            self.scanners = Some(vec![(DUMMY_SCANNER.into(), extra().demo_scanner.into())]);
        }
    }

    fn effective_dark(&self, ctx: &egui::Context) -> bool {
        match self.theme {
            1 => true,
            2 => false,
            _ => ctx.system_theme() != Some(egui::Theme::Light),
        }
    }

    fn palette(&self, ctx: &egui::Context) -> Palette {
        if self.effective_dark(ctx) {
            Palette::dark()
        } else {
            Palette::light()
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context, force: bool) {
        let dark = self.effective_dark(ctx);
        if force || self.applied_dark != Some(dark) {
            self.applied_dark = Some(dark);
            apply_visuals(ctx, if dark { Palette::dark() } else { Palette::light() });
        }
    }

    fn config_text(&self) -> String {
        let mut text = format!(
            "printer={}\ncolor={}\nduplex={}\nreverse_back={}\nreverse={}\npaper={}\ncopies={}\nper_sheet={}\nscale={}\nquality={}\n\
             scanner={}\nscan_source={}\nscan_mode={}\nscan_dpi={}\nscan_format={}\n\
             lang={}\ntheme={}\nvolume={}\nmascot={}\ngraphics={}\nscan_folder={}\n",
            self.printers.get(self.printer).map_or("", String::as_str),
            self.color,
            self.duplex,
            self.reverse_back,
            self.reverse,
            PAPERS[self.paper],
            self.copies,
            PER_SHEET[self.per_sheet],
            SCALES[self.scale],
            QUALITIES[self.quality],
            self.scanners
                .as_ref()
                .and_then(|l| l.get(self.scanner))
                .map_or(self.scanner_pref.as_str(), |(d, _)| d.as_str()),
            SCAN_SOURCES[self.scan_source],
            SCAN_MODES[self.scan_mode],
            SCAN_DPI[self.scan_dpi],
            SCAN_FORMATS[self.scan_format],
            self.lang,
            pet::THEME_NAMES[self.theme],
            sound::volume(),
            self.mascot,
            self.graphics_pref,
            self.scan_folder,
        );
        for f in &self.recent {
            text.push_str(&format!("recent={f}\n"));
        }
        text
    }

    fn save(&mut self) -> std::io::Result<()> {
        let path = config_path().ok_or(std::io::Error::other("Config path unavailable"))?;
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
        let text = self.config_text();
        self.config_cache = text.clone();
        std::fs::write(path, text)
    }

    fn save_settings(&mut self) {
        if self.dummy {
            self.config_cache = self.config_text();
            return;
        }
        if let Err(e) = self.save() {
            self.status = fill(t().settings_not_saved, &[("e", &e)]);
        }
    }

    fn hop(&mut self, row: usize) {
        self.focused_row = row;
        self.act = Act::Hop(Instant::now());
        self.hop_row = Some((row, Instant::now()));
        if !self.status.starts_with(t().error) {
            self.status.clear();
        }
        sound::play(Sound::Blip);
    }

    fn is_hopping(&self, row: usize) -> bool {
        self.hop_row
            .is_some_and(|(r, at)| r == row && at.elapsed() < Duration::from_millis(280))
    }

    fn spawn(&mut self, msg: impl Into<String>, work: impl FnOnce() -> Done + Send + 'static) {
        self.busy = Some((msg.into(), task(work)));
    }

    fn printer_state(&self) -> Option<&PrinterState> {
        self.state
            .as_ref()
            .filter(|(q, _)| self.printers.get(self.printer) == Some(q))
            .map(|(_, s)| s)
    }

    fn auto_duplex(&self) -> bool {
        !self.dummy
            && self.duplex
            && self.duplex_caps.get(self.printer).copied().unwrap_or(false)
    }

    fn mood(&self) -> Mood {
        if self.printers.is_empty() && self.backend.is_some() {
            return Mood::Off;
        }
        let problems = self.printer_state().map_or(&[][..], |s| s.problems.as_slice());
        let has = |names: &[&str]| problems.iter().any(|p| names.contains(&p.as_str()));
        if has(&[
            "offline",
            "shutdown",
            "connecting-to-device",
            "timed-out",
            "stopped",
            "paused",
        ]) {
            Mood::Off
        } else if has(&["media-empty", "media-needed"]) {
            Mood::NoPaper
        } else if problems.is_empty() {
            Mood::Fine
        } else {
            Mood::Trouble
        }
    }

    fn complaint(&self) -> Option<String> {
        if self.printers.is_empty() {
            return backend_text(self.backend);
        }
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

    fn chat(&self) -> String {
        let n = |template: &str, v: &dyn std::fmt::Display| fill(template, &[("n", v)]);
        match self.tab {
            Tab::Settings => match self.focused_row {
                C_LANG => t().lang_chat.into(),
                C_THEME => t().theme_chat.into(),
                C_VOLUME if sound::volume() == 0 => t().muted_chat.into(),
                C_VOLUME => t().volume_chat.into(),
                C_MASCOT => t().mascot_chat.into(),
                _ => t().folder_chat.into(),
            },
            Tab::Scan => {
                let kept = self.scans.iter().filter(|p| p.keep).count();
                let feeder = self.scan_source == 1;
                match self.focused_row {
                    S_SCANNER if self.scanners.as_ref().is_some_and(Vec::is_empty) => {
                        t().no_scanner_yet.into()
                    }
                    S_SCANNER => t().search_again.into(),
                    S_SOURCE if feeder => t().source_chat.into(),
                    S_SOURCE => t().scan_chat.into(),
                    S_MODE => fill(t().mode_chat, &[("mode", &t().scan_modes[self.scan_mode])]),
                    S_DPI if SCAN_DPI[self.scan_dpi] >= 600 => {
                        n(t().dpi_high, &SCAN_DPI[self.scan_dpi])
                    }
                    S_DPI => n(t().dpi_chat, &SCAN_DPI[self.scan_dpi]),
                    S_FORMAT if SCAN_FORMATS[self.scan_format] == "OCR" => t().ocr_chat.into(),
                    S_FORMAT => t().format_chat.into(),
                    S_SAVE_AS => t().save_as_chat.into(),
                    S_SCAN if feeder => t().source_chat.into(),
                    S_SCAN => t().scan_chat.into(),
                    S_PAGE if self.scans.is_empty() => t().pages_here.into(),
                    S_PAGE => t().page_keys.into(),
                    S_SAVE if kept == 0 => t().nothing_to_save.into(),
                    S_SAVE if kept == 1 => t().ready_one.into(),
                    S_SAVE => n(t().ready_many, &kept),
                    S_COPY if self.scans.is_empty() => t().copy_chat.into(),
                    S_COPY if kept == 1 => t().copy_one_chat.into(),
                    S_COPY => n(t().copy_many_chat, &kept),
                    _ => t().discard_chat.into(),
                }
            }
            Tab::Print => match self.focused_row {
                R_PRINTER if self.printers.is_empty() => t().no_printer_yet.into(),
                R_PRINTER if self.printer_state().is_some_and(|s| s.ink.iter().any(|i| i.2)) => {
                    t().ink_low.into()
                }
                R_PRINTER => fill(
                    t().at_service,
                    &[(
                        "name",
                        &self
                            .labels
                            .get(self.printer)
                            .map_or(t().your_printer, String::as_str),
                    )],
                ),
                R_FILE if self.file.is_empty() => t().feed_me.into(),
                R_FILE => t().other_files.into(),
                R_COLOR if self.color => t().colors.into(),
                R_COLOR => t().grayscale_chat.into(),
                R_SIDES if self.duplex => t().duplex_chat.into(),
                R_SIDES => t().one_side.into(),
                R_ORDER if !self.duplex => t().order_chat.into(),
                R_ORDER => t().back_order_chat.into(),
                R_PAGES => t().pages_chat.into(),
                R_PAPER => fill(t().paper_chat, &[("paper", &PAPERS[self.paper])]),
                R_COPIES if self.copies >= 10 => n(t().copies_lots, &self.copies),
                R_COPIES if self.copies == 1 => t().copies_one.into(),
                R_COPIES => n(t().copies_many, &self.copies),
                R_PER_SHEET if PER_SHEET[self.per_sheet] > 1 => {
                    n(t().per_sheet_many, &PER_SHEET[self.per_sheet])
                }
                R_PER_SHEET => t().per_sheet_one.into(),
                R_SCALE => match SCALES[self.scale] {
                    100 => t().scale_same.into(),
                    s if s > 100 => n(t().scale_big, &s),
                    s => n(t().scale_small, &s),
                },
                R_QUALITY => t().quality_chat[self.quality].into(),
                R_PRINT => t().print_chat.into(),
                R_ADD => t().add_chat.into(),
                _ => t().queue_chat.into(),
            },
        }
    }

    fn job(&self, file: &str, pages: Option<String>, reverse: bool, collate: bool) -> Job {
        Job {
            printer: self.printers[self.printer].clone(),
            file: file.into(),
            color: self.color,
            sides: false,
            paper: PAPERS[self.paper],
            pages,
            reverse,
            copies: self.copies.max(1),
            collate,
            per_sheet: PER_SHEET[self.per_sheet],
            quality: 3 + self.quality as u32,
        }
    }

    fn files(&self) -> Result<Vec<String>, String> {
        self.printers
            .get(self.printer)
            .ok_or(t().no_printer_selected)?;
        let files: Vec<String> = split_files(&self.file)
            .iter()
            .map(|f| expand_home(f))
            .collect();
        if files.is_empty() {
            return Err(t().no_file_selected.into());
        }
        if let Some(f) = files.iter().find(|f| !std::path::Path::new(f).is_file()) {
            return Err(fill(t().file_not_found, &[("file", f)]));
        }
        Ok(files)
    }

    fn print_now_headless(&mut self) -> Result<String, String> {
        if self.printers.is_empty()
            && let Some(msg) = backend_text(self.backend)
        {
            return Err(msg);
        }
        let files = self.files()?;
        let (scale, paper) = (SCALES[self.scale], PAPERS[self.paper]);
        let auto = self.auto_duplex();
        if self.duplex && !auto {
            return Err("Manual double-sided needs the app: open PrinterTUI and print there.".into());
        }
        let pdfs: Vec<String> = files
            .iter()
            .map(|f| printable(f, scale, paper))
            .collect::<Result<_, _>>()?;
        let jobs = self.prepare_batch(pdfs, auto, true)?;
        let mut out = Vec::new();
        for (job, _) in &jobs {
            out.push(submit(job)?);
        }
        Ok(out.join("\n"))
    }

    fn prepare_batch(
        &self,
        pdfs: Vec<String>,
        auto: bool,
        use_range: bool,
    ) -> Result<Vec<(Job, usize)>, String> {
        let pages = if use_range { self.pages.trim() } else { "" };
        let all = pages.is_empty() || pages == "all";
        let pdfs: Vec<String> = if self.reverse && !auto {
            pdfs.into_iter().rev().collect()
        } else {
            pdfs
        };
        let mut jobs = Vec::with_capacity(pdfs.len());
        for p in &pdfs {
            let total = page_count(p);
            let range = if all {
                None
            } else {
                Some(parse_ranges(pages, total.ok_or(t().unreadable_pdf)?)?)
            };
            let n = range.as_ref().map_or(total.unwrap_or(1) as usize, Vec::len);
            let mut job = self.job(p, range.map(|r| join(&r)), self.reverse && !auto, true);
            job.sides = auto;
            jobs.push((job, n));
        }
        Ok(jobs)
    }

    fn remember(&mut self, files: &[String]) {
        for f in files.iter().rev() {
            self.recent.retain(|r| r != f);
            self.recent.insert(0, f.clone());
        }
        self.recent.truncate(RECENT);
    }

    fn submit_job(&mut self, job: &Job, pages: usize) -> Result<String, String> {
        if !self.dummy {
            return submit(job);
        }
        let id = format!("{DUMMY_QUEUE}-{}", self.dummy_next_job);
        self.dummy_next_job += 1;
        let sides = pages.div_ceil(job.per_sheet as usize).max(1) as u32 * job.copies;
        self.dummy_jobs.push(DummyJob {
            id: id.clone(),
            description: format!("{} · {}", name(&job.file), extra().simulated),
            sides,
            started: Instant::now(),
            cancelled: false,
        });
        Ok(format!("request id is {id} ({})", job.printer))
    }

    fn follow(&mut self, lp_out: &str, job: &Job, pages: usize) {
        if let Some(id) = job_id(lp_out) {
            self.printing.push((
                id.into(),
                pages.div_ceil(job.per_sheet as usize) as u32 * job.copies,
                0,
            ));
        }
    }

    fn send(&mut self, job: &Job, pages: usize) -> Result<String, String> {
        let out = self.submit_job(job, pages)?;
        self.follow(&out, job, pages);
        Ok(out)
    }

    fn show(&mut self, res: Result<String, String>) {
        if res.is_ok() {
            self.act = Act::Print(Instant::now(), if self.printing.is_empty() { 1 } else { 0 });
            sound::play(Sound::Print);
        }
        self.status = match res {
            Ok(s) if self.dummy => {
                self.config_cache = self.config_text();
                s
            }
            Ok(s) => match self.save() {
                Ok(()) => s,
                Err(e) => format!("{s}\n\n{}", fill(t().settings_not_saved, &[("e", &e)]))
                    .trim_start()
                    .into(),
            },
            Err(e) => failed(e),
        };
    }

    fn try_print(&mut self) {
        if self.busy.is_some() {
            return;
        }
        let files = match self.files() {
            Ok(f) => f,
            Err(e) => return self.show(Err(e)),
        };
        let (scale, paper) = (SCALES[self.scale], PAPERS[self.paper]);
        self.spawn(t().preparing, move || {
            let pdfs: Result<Vec<String>, String> =
                files.iter().map(|f| printable(f, scale, paper)).collect();
            Box::new(move |app: &mut GuiApp| {
                let res = pdfs.and_then(|p| app.send_pdfs(p, true));
                if res.is_ok() {
                    app.remember(&files);
                }
                app.show(res);
            })
        });
    }

    fn send_pdfs(&mut self, pdfs: Vec<String>, use_range: bool) -> Result<String, String> {
        let auto = self.auto_duplex();
        if self.duplex && !auto {
            let range_str = if use_range { self.pages.as_str() } else { "" };
            for f in &pdfs {
                let total = page_count(f).ok_or(t().unreadable_pdf)?;
                parse_ranges(range_str, total)?;
            }
            return self.duplex_batch(pdfs, 0, String::new(), use_range);
        }
        for (job, n) in self.prepare_batch(pdfs, auto, use_range)? {
            self.send(&job, n)?;
        }
        Ok(String::new())
    }

    fn duplex_batch(
        &mut self,
        files: Vec<String>,
        mut i: usize,
        mut done: String,
        use_range: bool,
    ) -> Result<String, String> {
        self.modal = Modal::None;
        while let Some(file) = files.get(i) {
            i += 1;
            let head = if files.len() > 1 {
                fill(
                    t().file_of,
                    &[("i", &i), ("n", &files.len()), ("name", &name(file))],
                ) + "\n"
            } else {
                String::new()
            };
            let pdf = file.clone();
            let total = page_count(&pdf).ok_or(t().unreadable_pdf)?;
            let pages = parse_ranges(if use_range { &self.pages } else { "" }, total)?;
            let per_sheet = PER_SHEET[self.per_sheet] as usize;
            let (pdf, front, back) = prepare_duplex(&pdf, total, &pages, per_sheet)?;
            let id = self.send(&self.job(&pdf, Some(join(&front)), false, true), front.len())?;
            if back.is_empty() {
                done += &format!("{head}{}\n\n", fill(t().one_page, &[("id", &job_no(&id))]));
                continue;
            }
            let sent = fill(t().front_sent, &[("id", &job_no(&id))]);
            let steps = format!("{done}{head}{sent}\n\n{}", t().flip_steps);
            let wait = format!("{done}{head}{sent}\n\n{}", t().printing_front);
            let job = Some(job_id(&id).unwrap_or_default().to_string());
            let back = (
                self.job(&pdf, Some(join(&back)), self.reverse_back, true),
                back.len(),
            );
            self.modal = Modal::Flip {
                job,
                steps,
                back,
                files,
                next: i,
                use_range,
            };
            return Ok(wait);
        }
        Ok(done.trim_end().to_string())
    }

    fn back_side(&mut self) -> Result<String, String> {
        let Modal::Flip {
            back,
            files,
            next,
            use_range,
            ..
        } = std::mem::replace(&mut self.modal, Modal::None)
        else {
            return Ok(String::new());
        };
        let id = self.send(&back.0, back.1)?;
        self.duplex_batch(
            files,
            next,
            fill(t().back_sent, &[("id", &job_no(&id))]) + "\n\n",
            use_range,
        )
    }

    fn apply_job_states(&mut self, states: Vec<(i32, u32)>) {
        if states.iter().all(|(state, _)| *state >= 7) {
            let ok = states.iter().all(|(state, _)| *state == 9);
            if self.dummy {
                let done: Vec<String> = self.printing.iter().map(|(id, ..)| id.clone()).collect();
                self.dummy_jobs.retain(|job| !done.contains(&job.id));
            }
            self.printing.clear();
            self.act = if ok {
                Act::Print(Instant::now(), 1)
            } else {
                Act::Idle
            };
            if ok {
                self.checked = Instant::now() - Duration::from_secs(20);
                if !matches!(self.modal, Modal::Flip { .. }) && !self.status.starts_with(t().error) {
                    self.status = if self.dummy {
                        extra().demo_done.into()
                    } else {
                        t().printed.into()
                    };
                    sound::play(Sound::Done);
                }
            }
            return;
        }
        for ((_, sides, done), (state, n)) in self.printing.iter_mut().zip(states) {
            *done = if state >= 7 { *sides } else { n.min(*sides) };
        }
    }

    fn open_pages(&mut self) {
        if self.busy.is_some() {
            return;
        }
        let files: Vec<String> = split_files(&self.file)
            .iter()
            .map(|f| expand_home(f))
            .collect();
        if files.is_empty() {
            self.status = failed(t().pick_file_first);
            return;
        }
        let paper = PAPERS[self.paper];
        self.spawn(t().reading_pages, move || {
            let total = files
                .iter()
                .map(|f| {
                    printable(f, 100, paper)
                        .and_then(|p| page_count(&p).ok_or_else(|| t().unreadable_pdf.into()))
                })
                .collect::<Result<Vec<_>, String>>()
                .map(|counts| counts.into_iter().min().unwrap_or(1));
            Box::new(move |app: &mut GuiApp| match total {
                Ok(total) => {
                    let current = parse_ranges(&app.pages, total).unwrap_or_default();
                    let on = (1..=total).map(|n| current.contains(&n)).collect();
                    app.status.clear();
                    app.modal = Modal::Pages(on);
                }
                Err(e) => app.status = failed(e),
            })
        });
    }

    fn open_queue(&mut self) {
        if self.dummy {
            self.modal = Modal::Queue(dummy_queue(&self.dummy_jobs));
            return;
        }
        self.modal = Modal::Queue(Vec::new());
        self.polling = Some(task(|| {
            let jobs = queue();
            Box::new(move |app: &mut GuiApp| {
                if let Modal::Queue(ref mut q) = app.modal {
                    *q = jobs;
                }
            })
        }));
    }

    fn open_add_printer(&mut self) {
        if self.dummy {
            self.status = extra().demo_no_add.into();
            return;
        }
        if self.busy.is_some() {
            return;
        }
        self.spawn(t().searching_printers, || {
            let found = discover();
            Box::new(move |app: &mut GuiApp| {
                if found.is_empty() {
                    app.status = t().no_net_printers.into();
                } else {
                    app.status.clear();
                }
                app.modal = Modal::AddPrinter {
                    found,
                    address: String::new(),
                };
            })
        });
    }

    fn add_network_printer(&mut self, name: String, uri: String) {
        if self.dummy {
            self.status = extra().demo_no_add.into();
            self.modal = Modal::None;
            return;
        }
        self.modal = Modal::None;
        let msg = fill(
            t().adding,
            &[("name", &name), ("uri", &uri), ("note", &t().hints()[2])],
        );
        self.spawn(msg, move || {
            let res = add_printer(&name, &uri);
            let backend = backend_problem();
            let list = printers();
            Box::new(move |app: &mut GuiApp| {
                app.backend = backend;
                app.set_printers(list);
                if let Some(pos) = app.printers.iter().position(|p| p == &name) {
                    app.printer = pos;
                }
                app.status = match res {
                    Ok(()) => {
                        sound::play(Sound::Done);
                        fill(t().added, &[("name", &name)])
                    }
                    Err(e) => failed(e),
                };
            })
        });
    }

    fn open_doctor(&mut self) {
        if self.dummy {
            self.modal = Modal::Doctor(format!("{}\n\n{}", extra().demo_status, extra().doc_ok));
            return;
        }
        if self.busy.is_some() {
            return;
        }
        let current = self.printers.get(self.printer).cloned();
        let current_label = self.labels.get(self.printer).cloned();
        self.spawn(extra().checking_doctor, move || {
            let (backend, usb, net, legacy) = doctor_scan();
            let state = current.as_deref().map(printer_state);
            Box::new(move |app: &mut GuiApp| {
                app.backend = backend;
                let mut lines = Vec::new();
                if let Some(msg) = backend_text(backend) {
                    lines.push(format!("✗ {msg}"));
                } else {
                    lines.push(extra().doc_ok.to_string());
                }
                if let (Some(label), Some(st)) = (&current_label, &state) {
                    let src = match st.ink_source {
                        InkSource::Ipp => "IPP",
                        InkSource::CupsCache => "CUPS",
                        InkSource::Demo => "DEMO",
                        InkSource::Unknown => "?",
                    };
                    lines.push(format!("• {label} ({}: {src})", t().ink));
                    app.state = current.map(|q| (q, st.clone()));
                    if let Some(problem) = app.complaint() {
                        lines.push(format!("  ⚠ {problem}"));
                    }
                }
                if !usb.is_empty() {
                    lines.push(fill(extra().doc_usb, &[("list", &usb.join(", "))]));
                }
                if !net.is_empty() {
                    lines.push(fill(extra().doc_net, &[("n", &net.len())]));
                    for (n, u) in net.iter().take(4) {
                        lines.push(format!("  {n}  ({u})"));
                    }
                } else if app.printers.is_empty() && usb.is_empty() && backend.is_none() {
                    lines.push(extra().doc_no_net.to_string());
                }
                if !legacy.is_empty() {
                    lines.push(fill(extra().doc_legacy, &[("list", &legacy.join(", "))]));
                }
                app.modal = Modal::Doctor(lines.join("\n\n"));
            })
        });
    }

    fn refresh_devices(&mut self) {
        if self.dummy {
            self.hop(0);
            return;
        }
        self.backend = backend_problem();
        let prev = self.printers.get(self.printer).cloned();
        self.set_printers(printers());
        if let Some(p) = prev
            && let Some(idx) = self.printers.iter().position(|x| *x == p)
        {
            self.printer = idx;
        }
        self.checked = Instant::now() - Duration::from_secs(30);
        if self.tab == Tab::Scan {
            self.find_scanners(true);
        } else {
            self.hop(R_PRINTER);
        }
    }

    fn find_scanners(&mut self, full: bool) {
        if self.dummy {
            self.scanners = Some(vec![(DUMMY_SCANNER.into(), extra().demo_scanner.into())]);
            self.scanner = 0;
            return;
        }
        if self.busy.is_some() {
            return;
        }
        self.spawn(t().searching_scanners, move || {
            let list = scanners(full);
            Box::new(move |app: &mut GuiApp| {
                app.scanner = list
                    .iter()
                    .position(|(d, _)| *d == app.scanner_pref)
                    .unwrap_or(0);
                app.status = if list.is_empty() {
                    fill(t().no_scanners, &[("hint", &t().hints()[1])])
                } else {
                    String::new()
                };
                app.scanners = Some(list);
            })
        });
    }

    fn scan_page(&mut self, then_copy: bool, retake: Option<String>) {
        if self.busy.is_some() {
            return;
        }
        let Some((device, _)) = self
            .scanners
            .as_ref()
            .and_then(|l| l.get(self.scanner))
            .cloned()
        else {
            self.status = failed(t().no_scanner_selected);
            return;
        };
        let dir = std::env::temp_dir().join("printertui-scan");
        let n = retake
            .as_ref()
            .and_then(|o| self.scans.iter().position(|p| p.orig == *o))
            .unwrap_or(self.scans.len())
            + 1;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let (mode, dpi, paper) = (
            SCAN_MODES[self.scan_mode],
            SCAN_DPI[self.scan_dpi],
            PAPERS[self.paper],
        );
        let source = SCAN_SOURCES[self.scan_source];
        let stack = source == "Feeder" && retake.is_none();
        self.scanning = true;
        sound::play(Sound::Scan);
        self.spawn(fill(t().scanning, &[("n", &n)]), move || {
            let res = std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .and_then(|_| {
                    scan_pages(&device, mode, dpi, paper, source, stack, |i| {
                        let fname = if i == 0 {
                            format!("page-{stamp}")
                        } else {
                            format!("page-{stamp}-{i}")
                        };
                        dir.join(fname).to_string_lossy().into_owned()
                    })
                })
                .and_then(|paths| {
                    paths
                        .into_iter()
                        .map(|path| decode_color_image(&path).map(|img| (path, img)))
                        .collect::<Result<Vec<_>, _>>()
                });
            Box::new(move |app: &mut GuiApp| {
                app.scanning = false;
                app.status = match res {
                    Ok(pages) => {
                        sound::play(Sound::Done);
                        let m = pages.len();
                        let mut pages = pages.into_iter().map(|(path, image)| ScannedPage {
                            orig: path.clone(),
                            file: path,
                            rot: 0,
                            filter: 0,
                            keep: true,
                            image,
                            texture: None,
                        });
                        if let Some(i) = retake
                            .and_then(|o| app.scans.iter().position(|p| p.orig == o))
                        {
                            if let Some(page) = pages.next() {
                                let old_orig = std::mem::replace(&mut app.scans[i], page).orig;
                                if !app.discarded.iter().any(|p| p.orig == old_orig) {
                                    clean_scan_files(&[old_orig]);
                                }
                                app.cur = i;
                            }
                            return app.status = fill(t().page_rescanned, &[("n", &(i + 1))]);
                        }
                        app.scans.extend(pages);
                        app.cur = app.scans.len().saturating_sub(1);
                        if then_copy {
                            return app.copy();
                        }
                        if stack {
                            match m {
                                1 => t().ready_one.into(),
                                m => fill(t().ready_many, &[("n", &m)]),
                            }
                        } else {
                            fill(t().scanned, &[("n", &n)])
                        }
                    }
                    Err(e) => failed(e),
                };
            })
        });
    }

    fn edit_current_scan(&mut self) {
        let Some(p) = self.scans.get(self.cur) else {
            return;
        };
        let (orig, rot, filter) = (p.orig.clone(), p.rot, p.filter);
        self.editing = Some(task(move || {
            let res = edit_page(&orig, rot, filter)
                .and_then(|(file, _)| decode_color_image(&file).map(|img| (file, img)));
            Box::new(move |app: &mut GuiApp| match res {
                Ok((file, image)) => {
                    if let Some(p) = app
                        .scans
                        .iter_mut()
                        .find(|p| p.orig == orig && (p.rot, p.filter) == (rot, filter))
                    {
                        p.file = file;
                        p.image = image;
                        p.texture = None;
                    }
                }
                Err(e) => app.status = failed(e),
            })
        }));
    }

    fn save_scans(&mut self) {
        if self.busy.is_some() {
            return;
        }
        let files: Vec<String> = self
            .scans
            .iter()
            .filter(|p| p.keep)
            .map(|p| p.file.clone())
            .collect();
        if files.is_empty() {
            self.status = failed(t().no_pages_save);
            return;
        }
        if self.editing.is_some() {
            self.status = t().edit_running.into();
            return;
        }
        let origs: Vec<String> = self.scans.iter().map(|p| p.orig.clone()).collect();
        let out = expand_home(self.save_as.trim());
        let out = out
            .trim_end_matches(".pdf")
            .trim_end_matches(".png")
            .to_string();
        let (format, dpi) = (SCAN_FORMATS[self.scan_format], SCAN_DPI[self.scan_dpi]);
        let lang = t().code;
        self.spawn(t().saving, move || {
            let res = save_scans(&files, &out, format, dpi, lang);
            Box::new(move |app: &mut GuiApp| {
                app.status = match res {
                    Ok(written) => {
                        let clean: Vec<String> = origs
                            .into_iter()
                            .filter(|o| !app.discarded.iter().any(|p| p.orig == *o))
                            .collect();
                        clean_scan_files(&clean);
                        app.scans.clear();
                        app.cur = 0;
                        app.save_as = default_scan_name(&app.scan_folder);
                        if !app.dummy {
                            let _ = app.save();
                        }
                        sound::play(Sound::Done);
                        let written: Vec<String> = written.iter().map(|p| tilde(p)).collect();
                        fill(t().saved, &[("files", &written.join(", "))])
                    }
                    Err(e) => failed(e),
                };
            })
        });
    }

    fn copy(&mut self) {
        if self.busy.is_some() {
            return;
        }
        if self.printers.is_empty() {
            self.status = failed(t().no_printer_for_copy);
            return;
        }
        if self.scans.is_empty() {
            self.scan_page(true, None);
            return;
        }
        let files: Vec<String> = self
            .scans
            .iter()
            .filter(|p| p.keep)
            .map(|p| p.file.clone())
            .collect();
        if files.is_empty() {
            self.status = failed(t().no_pages_copy);
            return;
        }
        if self.editing.is_some() {
            self.status = t().edit_running.into();
            return;
        }
        let pdf = std::env::temp_dir()
            .join("printertui-scan")
            .join("copy.pdf");
        let (dpi, scale, paper) = (
            SCAN_DPI[self.scan_dpi],
            SCALES[self.scale],
            PAPERS[self.paper],
        );
        self.spawn(t().printing_copy, move || {
            let res = images_to_pdf(&files, dpi, &[])
                .and_then(|data| {
                    if let Some(parent) = pdf.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::write(&pdf, data).map_err(|e| format!("{}: {e}", pdf.display()))
                })
                .and_then(|_| printable(&pdf.to_string_lossy(), scale, paper));
            Box::new(move |app: &mut GuiApp| {
                let res = res.and_then(|file| app.send_pdfs(vec![file], false));
                app.show(res);
            })
        });
    }

    fn discard_scans(&mut self) {
        let old: Vec<String> = self
            .discarded
            .drain(..)
            .map(|p| p.orig)
            .filter(|o| !self.scans.iter().any(|p| p.orig == *o))
            .collect();
        clean_scan_files(&old);
        self.discarded = std::mem::take(&mut self.scans);
        self.cur = 0;
        self.status = t().discarded.into();
    }

    fn undo_discard(&mut self) {
        if self.scans.is_empty() && !self.discarded.is_empty() {
            self.scans = std::mem::take(&mut self.discarded);
            self.cur = self.scans.len().saturating_sub(1);
            self.status = t().loaded_last.into();
            sound::play(Sound::Blip);
        }
    }

    fn delete_current_scan(&mut self) {
        if self.scans.is_empty() {
            return;
        }
        let i = self.cur.min(self.scans.len() - 1);
        let removed = self.scans.remove(i);
        if !self.discarded.iter().any(|p| p.orig == removed.orig) {
            clean_scan_files(&[removed.orig]);
        }
        self.cur = i.min(self.scans.len().saturating_sub(1));
        self.status = fill(t().page_deleted, &[("n", &(i + 1))]);
        sound::play(Sound::Blip);
    }

    fn step_preview(&mut self, fwd: bool) {
        if self.tab == Tab::Scan && !self.scans.is_empty() {
            let n = self.scans.len();
            self.cur = (self.cur + if fwd { 1 } else { n - 1 }) % n;
            return;
        }
        if self.tab == Tab::Print {
            let (idx, count) = match &self.view {
                Some((_, Ok(v))) => (v.idx, v.count),
                _ => (0, 1),
            };
            let files = split_files(&self.file).len().max(1);
            if fwd {
                if idx + 1 < count {
                    self.view_page = idx + 1;
                } else {
                    self.view_file = (self.view_file + 1) % files;
                    self.view_page = 0;
                }
            } else if idx > 0 {
                self.view_page = idx - 1;
            } else {
                self.view_file = (self.view_file + files - 1) % files;
                self.view_page = usize::MAX;
            }
        }
    }

    fn poll_background(&mut self) {
        if let Some(done) = self.busy.as_ref().and_then(|(_, rx)| finished(rx)) {
            self.busy = None;
            done(self);
        }
        if let Some(done) = self.editing.as_ref().and_then(finished) {
            self.editing = None;
            done(self);
        }
        if let Some(done) = self.viewing.as_ref().and_then(|(_, rx)| finished(rx)) {
            self.viewing = None;
            done(self);
        }
        if let Some(done) = self.checking.as_ref().and_then(finished) {
            self.checking = None;
            done(self);
        }
        if let Some(done) = self.polling.as_ref().and_then(finished) {
            self.polling = None;
            done(self);
        }
        // Printer state check every 20 seconds
        if !self.dummy
            && let Some(queue_name) = self.printers.get(self.printer).cloned()
        {
            let known = self.state.as_ref().is_some_and(|(q, _)| *q == queue_name);
            if self.checking.is_none()
                && (!known || self.checked.elapsed() >= Duration::from_secs(20))
            {
                self.checked = Instant::now();
                self.checking = Some(task(move || {
                    let st = printer_state(&queue_name);
                    Box::new(move |app: &mut GuiApp| app.state = Some((queue_name, st)))
                }));
            }
        }
        // Sync live Print preview
        if self.tab == Tab::Print {
            let files = split_files(&self.file);
            self.view_file = self.view_file.min(files.len().saturating_sub(1));
            if let Some(file) = files.get(self.view_file).map(|f| expand_home(f)) {
                let key = ViewKey {
                    file,
                    pages: self.pages.clone(),
                    scale: SCALES[self.scale],
                    paper: PAPERS[self.paper],
                    per_sheet: PER_SHEET[self.per_sheet],
                    idx: self.view_page,
                };
                if !self.view.as_ref().is_some_and(|(k, _)| *k == key) && self.viewing.is_none() {
                    let k = key.clone();
                    self.viewing = Some((
                        key,
                        task(move || {
                            let res = render_view(&k);
                            Box::new(move |app: &mut GuiApp| app.view = Some((k, res)))
                        }),
                    ));
                }
            } else {
                self.view = None;
            }
        }
        // Poll active print jobs / Queue modal / Flip front-side every second
        if self.last_poll.elapsed() >= Duration::from_secs(1) {
            self.last_poll = Instant::now();
            if self.dummy {
                if let Modal::Flip { job, steps, .. } = &mut self.modal
                    && job
                        .as_deref()
                        .is_some_and(|id| !dummy_job_active(&self.dummy_jobs, id))
                {
                    *job = None;
                    self.status = std::mem::take(steps);
                }
                if let Modal::Queue(jobs) = &mut self.modal {
                    *jobs = dummy_queue(&self.dummy_jobs);
                }
                if !self.printing.is_empty() {
                    let states: Vec<(i32, u32)> = self
                        .printing
                        .iter()
                        .map(|(id, ..)| match self.dummy_jobs.iter().find(|j| j.id == *id) {
                            Some(job) if job.cancelled => (7, job.sides),
                            Some(job) if job.active() => (5, job.progress()),
                            Some(job) => (9, job.sides),
                            None => (7, 0),
                        })
                        .collect();
                    self.apply_job_states(states);
                }
            } else if self.polling.is_none() {
                let flip_id = match &self.modal {
                    Modal::Flip { job: Some(id), .. } => Some(id.clone()),
                    _ => None,
                };
                let want_queue = matches!(self.modal, Modal::Queue(_));
                let ids: Vec<String> = self.printing.iter().map(|(id, ..)| id.clone()).collect();
                if flip_id.is_some() || want_queue || !ids.is_empty() {
                    self.polling = Some(task(move || {
                        let flip_done = flip_id.as_deref().is_some_and(|id| !job_active(id));
                        let q = want_queue.then(queue);
                        let states: Vec<(String, (i32, u32))> = ids
                            .into_iter()
                            .map(|id| {
                                let p = job_progress(&id);
                                (id, p)
                            })
                            .collect();
                        Box::new(move |app: &mut GuiApp| {
                            if flip_done
                                && let Modal::Flip { job, steps, .. } = &mut app.modal
                            {
                                *job = None;
                                app.status = std::mem::take(steps);
                            }
                            if let (Some(q), Modal::Queue(jobs)) = (q, &mut app.modal) {
                                *jobs = q;
                            }
                            if !app.printing.is_empty() && !states.is_empty() {
                                let mapped: Vec<(i32, u32)> = app
                                    .printing
                                    .iter()
                                    .map(|(id, _, done)| {
                                        states
                                            .iter()
                                            .find(|(sid, _)| sid == id)
                                            .map_or((5, *done), |(_, s)| *s)
                                    })
                                    .collect();
                                app.apply_job_states(mapped);
                            }
                        })
                    }));
                }
            }
        }
        // Keep paper flowing while jobs print
        if !self.printing.is_empty() && self.act.done() {
            self.act = Act::Print(Instant::now(), 0);
        }
        // Jam animation + sound on new error
        if self.status != self.heard {
            self.heard = self.status.clone();
            if self.status.starts_with(t().error) {
                self.act = Act::Jam(Instant::now());
                sound::play(Sound::Jam);
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = self.text_focused;
        let modal_open = !matches!(
            self.modal,
            Modal::None | Modal::Flip { job: Some(_), .. }
        );
        if modal_open {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                if matches!(self.modal, Modal::Flip { .. }) {
                    self.status = t().back_cancelled.into();
                }
                self.modal = Modal::None;
            }
            return;
        }
        // Ctrl+1/2/3 jump straight to Print/Scan/Settings even if a text field is focused
        let mut tab_switch = None;
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::COMMAND, Key::Num1) {
                tab_switch = Some(Tab::Print);
            } else if i.consume_key(egui::Modifiers::COMMAND, Key::Num2) {
                tab_switch = Some(Tab::Scan);
            } else if i.consume_key(egui::Modifiers::COMMAND, Key::Num3) {
                tab_switch = Some(Tab::Settings);
            } else if !typing && i.consume_key(egui::Modifiers::SHIFT, Key::Tab) {
                tab_switch = Some(match self.tab {
                    Tab::Print => Tab::Settings,
                    Tab::Scan => Tab::Print,
                    Tab::Settings => Tab::Scan,
                });
            } else if !typing && i.consume_key(egui::Modifiers::NONE, Key::Tab) {
                tab_switch = Some(match self.tab {
                    Tab::Print => Tab::Scan,
                    Tab::Scan => Tab::Settings,
                    Tab::Settings => Tab::Print,
                });
            }
        });
        if let Some(next) = tab_switch {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            self.switch_tab(next);
            return;
        }
        ctx.input(|i| {
            if i.modifiers.command && i.key_pressed(Key::O) {
                let picked = pick_files();
                if !picked.is_empty() {
                    self.file = picked.join("; ");
                    self.tab = Tab::Print;
                    self.view_file = 0;
                    self.view_page = 0;
                    self.hop(R_FILE);
                }
            }
            if i.modifiers.command && i.key_pressed(Key::P) {
                self.tab = Tab::Print;
                self.try_print();
            }
            if typing {
                return;
            }
            for ev in &i.events {
                let egui::Event::Text(text) = ev else { continue };
                match text.as_str() {
                    "?" => self.open_doctor(),
                    "H" => self.step_preview(false),
                    "L" => self.step_preview(true),
                    "r" if self.tab == Tab::Scan && !self.scans.is_empty() => {
                        self.scans[self.cur].rot = (self.scans[self.cur].rot + 90) % 360;
                        self.edit_current_scan();
                        self.hop(S_PAGE);
                    }
                    "R" if self.tab == Tab::Scan && !self.scans.is_empty() => {
                        self.scans[self.cur].rot = (self.scans[self.cur].rot + 270) % 360;
                        self.edit_current_scan();
                        self.hop(S_PAGE);
                    }
                    "f" if self.tab == Tab::Scan && !self.scans.is_empty() => {
                        self.scans[self.cur].filter =
                            (self.scans[self.cur].filter + 1) % FILTERS.len();
                        self.edit_current_scan();
                        self.hop(S_PAGE);
                    }
                    "x" if self.tab == Tab::Scan && !self.scans.is_empty() => {
                        self.scans[self.cur].keep = !self.scans[self.cur].keep;
                        self.hop(S_PAGE);
                    }
                    "<" if self.tab == Tab::Scan && self.cur > 0 => {
                        self.scans.swap(self.cur, self.cur - 1);
                        self.cur -= 1;
                        self.hop(S_PAGE);
                    }
                    ">" if self.tab == Tab::Scan && self.cur + 1 < self.scans.len() => {
                        self.scans.swap(self.cur, self.cur + 1);
                        self.cur += 1;
                        self.hop(S_PAGE);
                    }
                    "s" if self.tab == Tab::Scan && !self.scans.is_empty() => {
                        let orig = self.scans[self.cur].orig.clone();
                        self.scan_page(false, Some(orig));
                    }
                    "u" if self.tab == Tab::Scan => self.undo_discard(),
                    _ => {}
                }
            }
        });
    }

    fn handle_file_drop(&mut self, ctx: &egui::Context, p: Palette) {
        let (hovering, dropped) = ctx.input(|i| {
            let paths: Vec<String> = i
                .raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.as_ref())
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            (!i.raw.hovered_files.is_empty(), paths)
        });
        if !dropped.is_empty() {
            self.file = dropped.join("; ");
            self.tab = Tab::Print;
            self.view_file = 0;
            self.view_page = 0;
            self.hop(R_FILE);
        }
        if hovering {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(150));
            let card = screen.shrink(36.0);
            painter.rect_filled(card, CornerRadius::same(14), p.panel);
            painter.rect_stroke(
                card,
                CornerRadius::same(14),
                Stroke::new(2.0_f32, p.accent),
                StrokeKind::Inside,
            );
            painter.text(
                card.center(),
                egui::Align2::CENTER_CENTER,
                format!("▶  {}", t().feed_me),
                egui::FontId::proportional(20.0),
                p.text,
            );
        }
    }

    fn switch_tab(&mut self, next: Tab) {
        if self.tab == next {
            return;
        }
        self.tab = next;
        self.focused_row = 0;
        if !self.status.starts_with(t().error) {
            self.status.clear();
        }
        if next == Tab::Scan && self.scanners.is_none() {
            self.find_scanners(false);
        }
    }

    fn schedule_repaints(&self, ctx: &egui::Context) {
        let animating = self.busy.is_some()
            || self.editing.is_some()
            || self.viewing.is_some()
            || self.checking.is_some()
            || self.polling.is_some()
            || !self.printing.is_empty()
            || !self.act.done()
            || self.hop_row.is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(320))
            || matches!(self.modal, Modal::Flip { .. } | Modal::Queue(_));
        if animating {
            ctx.request_repaint_after(Duration::from_millis(65));
        } else if self.mascot {
            // Keep the mascot's gentle alternating green LED blink alive with minimal CPU
            ctx.request_repaint_after(Duration::from_millis(450));
        }
    }
}

impl eframe::App for GuiApp {
    fn on_exit(&mut self) {
        stop_all();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.intl_fonts_loaded && (self.tab == Tab::Settings || needs_intl_fonts(t().code)) {
            setup_fonts(ctx, true);
            self.intl_fonts_loaded = true;
        }
        self.apply_theme(ctx, false);
        self.poll_background();
        self.handle_shortcuts(ctx);
        self.text_focused = false;
        let p = self.palette(ctx);

        // Top header bar: PrinterTUI + Print | Scan | Settings + Doctor / Queue / Refresh
        egui::TopBottomPanel::top("header")
            .frame(
                Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .inner_margin(Margin::symmetric(14, 8)),
            )
            .show(ctx, |ui| self.draw_header(ui, p));

        // Bottom bar: key chips + Build #N · DD/MM/YYYY
        egui::TopBottomPanel::bottom("footer")
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .inner_margin(Margin::symmetric(14, 5)),
            )
            .show(ctx, |ui| self.draw_footer(ui, p));

        // Main two-column workspace: Left (Form + Mascot Stage), Right (Live Preview / Config)
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin::same(10)))
            .show(ctx, |ui| {
                let avail = ui.available_size();
                let gap = 10.0;
                let left_w = (avail.x * 0.50).clamp(468.0, 520.0).min(avail.x - 300.0);
                let right_w = (avail.x - left_w - gap).max(260.0);

                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;

                    // LEFT COLUMN: Form card on top, Mascot + Bubble + Ink Tanks below
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, avail.y),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            ui.set_max_width(left_w);
                            card_frame(p).show(ui, |ui| {
                                let left_inner = (left_w - 26.0).max(320.0);
                                ui.set_width(left_inner);
                                ui.set_max_width(left_inner);
                                ui.spacing_mut().item_spacing.y = 3.0;
                                match self.tab {
                                    Tab::Print => self.draw_print_form(ui, p),
                                    Tab::Scan => self.draw_scan_form(ui, p),
                                    Tab::Settings => self.draw_settings_form(ui, p),
                                }
                            });
                            ui.add_space(10.0);
                            self.draw_stage(ui, p);
                        },
                    );

                    // RIGHT COLUMN: Live full-height Preview / Config panel
                    ui.allocate_ui_with_layout(
                        Vec2::new(right_w, avail.y),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            ui.set_max_width(right_w);
                            card_frame(p).show(ui, |ui| {
                                let inner_w = (right_w - 26.0).max(220.0);
                                ui.set_width(inner_w);
                                ui.set_max_width(inner_w);
                                ui.set_min_size(Vec2::new(
                                    inner_w,
                                    (avail.y - 22.0).max(200.0),
                                ));
                                match self.tab {
                                    Tab::Print => self.draw_print_preview(ui, ctx, p),
                                    Tab::Scan => self.draw_scan_preview(ui, ctx, p),
                                    Tab::Settings => self.draw_config_panel(ui, p),
                                }
                            });
                        },
                    );
                });
            });

        self.draw_modal(ctx, p);
        self.handle_file_drop(ctx, p);
        self.schedule_repaints(ctx);
    }
}

fn card_frame(p: Palette) -> Frame {
    Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(12, 10))
}

impl GuiApp {
    fn draw_header(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            // Mini pixel-art emblem next to the title
            draw_mini_printer_badge(ui);
            ui.label(
                RichText::new("PrinterTUI")
                    .color(p.accent)
                    .strong()
                    .size(15.5),
            );
            ui.add_space(8.0);

            for (tab, label) in [
                (Tab::Print, t().tab_print),
                (Tab::Scan, t().tab_scan),
                (Tab::Settings, t().tab_settings),
            ] {
                let on = self.tab == tab;
                let btn = egui::Button::new(
                    RichText::new(label)
                        .color(if on { Color32::WHITE } else { p.muted })
                        .strong()
                        .size(13.5),
                )
                .fill(if on { p.accent } else { Color32::TRANSPARENT })
                .stroke(if on {
                    Stroke::NONE
                } else {
                    Stroke::new(1.0_f32, Color32::TRANSPARENT)
                })
                .corner_radius(CornerRadius::same(6));
                if ui.add_sized([0.0, 27.0], btn).clicked() {
                    self.switch_tab(tab);
                }
            }

            if self.dummy {
                ui.add_space(6.0);
                Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(
                        p.yellow.r(),
                        p.yellow.g(),
                        p.yellow.b(),
                        28,
                    ))
                    .stroke(Stroke::new(1.0_f32, p.yellow))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(extra().demo_banner)
                                .color(p.yellow)
                                .strong()
                                .size(11.5),
                        );
                    });
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if subtle_btn(ui, p, "↻")
                    .on_hover_text(t().search_again)
                    .clicked()
                {
                    self.refresh_devices();
                }
                if subtle_btn(ui, p, &format!("?  {}", extra().k_doctor)).clicked() {
                    self.open_doctor();
                }
                if !self.printing.is_empty()
                    && accent_outline_btn(
                        ui,
                        p,
                        &format!("{} ({})", t().btn_queue, self.printing.len()),
                        true,
                    )
                    .clicked()
                {
                    self.open_queue();
                }
            });
        });
    }

    fn draw_footer(&self, ui: &mut egui::Ui, p: Palette) {
        let t = t();
        let ex = extra();
        let hints: &[(&str, &str)] = match self.tab {
            Tab::Print => &[
                ("Tab", t.k_print_scan),
                ("Ctrl+O", t.labels[R_FILE]),
                ("H/L", t.k_preview_page),
                ("Ctrl+P", t.btn_print),
                ("?", ex.k_doctor),
            ],
            Tab::Scan if !self.scans.is_empty() => &[
                ("H/L", t.k_page),
                ("r/R", t.k_rotate),
                ("f", t.k_filter),
                ("x", t.k_keep),
                ("</>", t.k_move),
                ("s", t.k_retake),
            ],
            Tab::Scan if !self.discarded.is_empty() => {
                &[("Tab", t.k_print_scan), ("u", t.loaded_last), ("?", ex.k_doctor)]
            }
            Tab::Scan => &[("Tab", t.k_print_scan), ("?", ex.k_doctor)],
            Tab::Settings => &[("Tab", t.k_print_scan), ("?", ex.k_doctor)],
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (k, desc) in hints {
                Frame::new()
                    .fill(p.chip_bg)
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(5, 1))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(*k)
                                .color(p.chip_fg)
                                .monospace()
                                .size(11.0),
                        );
                    });
                ui.label(RichText::new(*desc).color(p.muted).size(11.5));
                ui.add_space(4.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(
                    RichText::new(env!("PRINTERTUI_BUILD_DATE"))
                        .color(p.muted)
                        .size(11.5),
                );
                ui.label(RichText::new("·").color(p.muted).size(11.5));
                ui.label(
                    RichText::new(format!("Build #{}", env!("PRINTERTUI_BUILD_NUMBER")))
                        .color(p.accent)
                        .strong()
                        .size(11.5),
                );
            });
        });
    }

    fn form_row(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        row: usize,
        label: &str,
        body: impl FnOnce(&mut Self, &mut egui::Ui, bool),
    ) {
        let active = self.focused_row == row;
        let hop = self.is_hopping(row);
        let resp = Frame::new()
            .fill(if active {
                p.accent_soft
            } else {
                Color32::TRANSPARENT
            })
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(24.0);
                    let (mark_rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 22.0), Sense::hover());
                    if active {
                        ui.painter().text(
                            mark_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "▶",
                            egui::FontId::proportional(10.5),
                            p.accent,
                        );
                    }
                    let lbl_color = if active { p.text } else { p.muted };
                    let (lbl_rect, _) = ui.allocate_exact_size(Vec2::new(110.0, 22.0), Sense::hover());
                    let clipped = ui.painter().with_clip_rect(lbl_rect);
                    clipped.text(
                        Pos2::new(lbl_rect.left(), lbl_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(13.0),
                        lbl_color,
                    );
                    body(self, ui, hop);
                });
            })
            .response;
        if resp.hovered() || resp.contains_pointer() {
            self.focused_row = row;
        }
    }

    fn draw_print_form(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = t();

        // 0: Printer
        self.form_row(ui, p, R_PRINTER, t.labels[R_PRINTER], |app, ui, hop| {
            let count = app.printers.len();
            if stepper_arrow(ui, p, "‹", count > 1).clicked() {
                app.printer = (app.printer + count - 1) % count;
                app.hop(R_PRINTER);
            }
            let current = app
                .labels
                .get(app.printer)
                .cloned()
                .unwrap_or_else(|| t.none.into());
            let w = (ui.available_width() - 32.0).max(120.0);
            let mut changed = false;
            egui::ComboBox::from_id_salt("printer_combo")
                .width(w)
                .selected_text(RichText::new(current).color(if hop { p.yellow } else { p.text }))
                .show_ui(ui, |ui| {
                    for (i, label) in app.labels.iter().enumerate() {
                        if ui.selectable_value(&mut app.printer, i, label).changed() {
                            changed = true;
                        }
                    }
                });
            if changed {
                app.hop(R_PRINTER);
            }
            if stepper_arrow(ui, p, "›", count > 1).clicked() {
                app.printer = (app.printer + 1) % count;
                app.hop(R_PRINTER);
            }
        });

        // 1: File (+ multi-file picker + recent dropdown)
        self.form_row(ui, p, R_FILE, t.labels[R_FILE], |app, ui, _| {
            let has_recent = !app.recent.is_empty();
            let extra_w = if has_recent { 68.0 } else { 38.0 };
            let edit_w = (ui.available_width() - extra_w).max(120.0);
            let resp = ui.add_sized(
                [edit_w, 24.0],
                egui::TextEdit::singleline(&mut app.file)
                    .hint_text(RichText::new(t.feed_me).color(p.muted))
                    .font(egui::TextStyle::Monospace),
            );
            app.text_focused |= resp.has_focus();
            if resp.changed() {
                app.view_file = 0;
                app.view_page = 0;
            }
            if subtle_btn(ui, p, "…").on_hover_text(t.k_select).clicked() {
                let picked = pick_files();
                if !picked.is_empty() {
                    app.file = picked.join("; ");
                    app.view_file = 0;
                    app.view_page = 0;
                    app.hop(R_FILE);
                }
            }
            if has_recent {
                let mb = ui.menu_button("   ", |ui| {
                    for recent_path in app.recent.clone() {
                        if ui.button(name(&recent_path)).clicked() {
                            app.file = recent_path;
                            app.view_file = 0;
                            app.view_page = 0;
                            app.hop(R_FILE);
                            ui.close_menu();
                        }
                    }
                });
                let c = mb.response.rect.center();
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(c.x - 4.0, c.y - 2.0),
                        Pos2::new(c.x + 4.0, c.y - 2.0),
                        Pos2::new(c.x, c.y + 3.0),
                    ],
                    p.accent,
                    Stroke::NONE,
                ));
            }
        });

        // 2: Color
        self.form_row(ui, p, R_COLOR, t.labels[R_COLOR], |app, ui, hop| {
            let mut idx = usize::from(app.color);
            if segmented(ui, p, &mut idx, &[t.grayscale, t.color], hop) {
                app.color = idx == 1;
                app.hop(R_COLOR);
            }
        });

        // 3: Sides
        self.form_row(ui, p, R_SIDES, t.labels[R_SIDES], |app, ui, hop| {
            let mut idx = usize::from(app.duplex);
            if segmented(ui, p, &mut idx, &[t.single_sided, t.double_sided], hop) {
                app.duplex = idx == 1;
                app.hop(R_SIDES);
            }
            if app.auto_duplex() {
                ui.label(RichText::new("auto").color(p.green).size(11.0).monospace());
            }
        });

        // 4: Order / Back order
        let order_label = if self.duplex { t.labels[R_ORDER] } else { t.order };
        self.form_row(ui, p, R_ORDER, order_label, |app, ui, hop| {
            let mut idx = usize::from(if app.duplex {
                app.reverse_back
            } else {
                app.reverse
            });
            if segmented(ui, p, &mut idx, &[t.normal, t.reversed], hop) {
                if app.duplex {
                    app.reverse_back = idx == 1;
                } else {
                    app.reverse = idx == 1;
                }
                app.hop(R_ORDER);
            }
        });

        // 5: Pages + visual page picker button
        self.form_row(ui, p, R_PAGES, t.labels[R_PAGES], |app, ui, _| {
            let edit_w = (ui.available_width() - 109.0).max(90.0);
            let resp = ui.add_sized(
                [edit_w, 24.0],
                egui::TextEdit::singleline(&mut app.pages)
                    .hint_text(RichText::new(t.all).color(p.muted))
                    .font(egui::TextStyle::Monospace),
            );
            app.text_focused |= resp.has_focus();
            if resp.changed() {
                app.view_page = 0;
            }
            if subtle_btn(ui, p, t.k_pick_pages).clicked() {
                app.open_pages();
            }
        });

        // 6: Paper
        self.form_row(ui, p, R_PAPER, t.labels[R_PAPER], |app, ui, hop| {
            if stepper_arrow(ui, p, "‹", true).clicked() {
                app.paper = (app.paper + PAPERS.len() - 1) % PAPERS.len();
                app.hop(R_PAPER);
            }
            let mut changed = false;
            egui::ComboBox::from_id_salt("paper_combo")
                .width(110.0)
                .selected_text(
                    RichText::new(PAPERS[app.paper]).color(if hop { p.yellow } else { p.text }),
                )
                .show_ui(ui, |ui| {
                    for (i, paper) in PAPERS.iter().enumerate() {
                        if ui.selectable_value(&mut app.paper, i, *paper).changed() {
                            changed = true;
                        }
                    }
                });
            if changed {
                app.hop(R_PAPER);
            }
            if stepper_arrow(ui, p, "›", true).clicked() {
                app.paper = (app.paper + 1) % PAPERS.len();
                app.hop(R_PAPER);
            }
        });

        // 7: Copies
        self.form_row(ui, p, R_COPIES, t.labels[R_COPIES], |app, ui, hop| {
            if stepper_arrow(ui, p, "−", app.copies > 1).clicked() {
                app.copies = app.copies.saturating_sub(1).max(1);
                app.hop(R_COPIES);
            }
            let prev = app.copies;
            ui.add_sized(
                [46.0, 24.0],
                egui::DragValue::new(&mut app.copies)
                    .range(1..=999)
                    .speed(0.1),
            );
            if app.copies != prev {
                app.hop(R_COPIES);
            }
            if stepper_arrow(ui, p, "+", app.copies < 999).clicked() {
                app.copies = (app.copies + 1).min(999);
                app.hop(R_COPIES);
            }
            if hop {
                ui.label(RichText::new(format!("×{}", app.copies)).color(p.yellow).size(12.0));
            }
        });

        // 8: Per sheet (1-click pills!)
        self.form_row(ui, p, R_PER_SHEET, t.labels[R_PER_SHEET], |app, ui, hop| {
            let mut idx = app.per_sheet;
            if segmented(ui, p, &mut idx, &["1", "2", "4", "6", "9", "16"], hop) {
                app.per_sheet = idx;
                app.view_page = 0;
                app.hop(R_PER_SHEET);
            }
        });

        // 9: Scale
        self.form_row(ui, p, R_SCALE, t.labels[R_SCALE], |app, ui, hop| {
            if stepper_arrow(ui, p, "‹", true).clicked() {
                app.scale = (app.scale + SCALES.len() - 1) % SCALES.len();
                app.hop(R_SCALE);
            }
            let mut changed = false;
            egui::ComboBox::from_id_salt("scale_combo")
                .width(88.0)
                .selected_text(
                    RichText::new(format!("{}%", SCALES[app.scale]))
                        .color(if hop { p.yellow } else { p.text }),
                )
                .show_ui(ui, |ui| {
                    for (i, s) in SCALES.iter().enumerate() {
                        if ui
                            .selectable_value(&mut app.scale, i, format!("{s}%"))
                            .changed()
                        {
                            changed = true;
                        }
                    }
                });
            if changed {
                app.hop(R_SCALE);
            }
            if stepper_arrow(ui, p, "›", true).clicked() {
                app.scale = (app.scale + 1) % SCALES.len();
                app.hop(R_SCALE);
            }
        });

        // 10: Quality
        self.form_row(ui, p, R_QUALITY, t.labels[R_QUALITY], |app, ui, hop| {
            let mut idx = app.quality;
            if segmented(ui, p, &mut idx, &t.qualities, hop) {
                app.quality = idx;
                app.hop(R_QUALITY);
            }
        });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(8.0);

        // Action row: Print (primary), Add printer, Print queue
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let busy = self.busy.is_some();
            let print_resp = primary_btn(ui, p, t.btn_print, !busy);
            if print_resp.hovered() {
                self.focused_row = R_PRINT;
            }
            if print_resp.clicked() {
                self.try_print();
            }

            let add_resp = accent_outline_btn(ui, p, t.btn_add, !busy);
            if add_resp.hovered() {
                self.focused_row = R_ADD;
            }
            if add_resp.clicked() {
                self.open_add_printer();
            }

            let q_resp = accent_outline_btn(ui, p, t.btn_queue, true);
            if q_resp.hovered() {
                self.focused_row = R_QUEUE;
            }
            if q_resp.clicked() {
                self.open_queue();
            }
        });
    }

    fn draw_scan_form(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = t();

        // 0: Scanner
        self.form_row(ui, p, S_SCANNER, t.scan_labels[0], |app, ui, hop| {
            let count = app.scanners.as_ref().map_or(0, Vec::len);
            if stepper_arrow(ui, p, "‹", count > 1).clicked() {
                app.scanner = (app.scanner + count - 1) % count;
                app.hop(S_SCANNER);
            }
            let current = app
                .scanners
                .as_ref()
                .and_then(|l| l.get(app.scanner))
                .map_or(t.none, |(_, d)| d.as_str())
                .to_string();
            let w = (ui.available_width() - 68.0).max(110.0);
            let mut changed = false;
            egui::ComboBox::from_id_salt("scanner_combo")
                .width(w)
                .selected_text(RichText::new(current).color(if hop { p.yellow } else { p.text }))
                .show_ui(ui, |ui| {
                    if let Some(list) = &app.scanners {
                        for (i, (_, desc)) in list.iter().enumerate() {
                            if ui.selectable_value(&mut app.scanner, i, desc).changed() {
                                changed = true;
                            }
                        }
                    }
                });
            if changed {
                app.hop(S_SCANNER);
            }
            if stepper_arrow(ui, p, "›", count > 1).clicked() {
                app.scanner = (app.scanner + 1) % count;
                app.hop(S_SCANNER);
            }
            if subtle_btn(ui, p, "↻")
                .on_hover_text(t.search_again)
                .clicked()
            {
                app.find_scanners(true);
            }
        });

        // 1: Source
        self.form_row(ui, p, S_SOURCE, t.scan_labels[1], |app, ui, hop| {
            let mut idx = app.scan_source;
            if segmented(ui, p, &mut idx, &t.scan_sources, hop) {
                app.scan_source = idx;
                app.hop(S_SOURCE);
            }
        });

        // 2: Mode
        self.form_row(ui, p, S_MODE, t.scan_labels[2], |app, ui, hop| {
            let mut idx = app.scan_mode;
            if segmented(ui, p, &mut idx, &t.scan_modes, hop) {
                app.scan_mode = idx;
                app.hop(S_MODE);
            }
        });

        // 3: Resolution
        self.form_row(ui, p, S_DPI, t.scan_labels[3], |app, ui, hop| {
            let mut idx = app.scan_dpi;
            if segmented(ui, p, &mut idx, &["150 dpi", "300 dpi", "600 dpi"], hop) {
                app.scan_dpi = idx;
                app.hop(S_DPI);
            }
        });

        // 4: Format
        self.form_row(ui, p, S_FORMAT, t.scan_labels[4], |app, ui, hop| {
            let mut idx = app.scan_format;
            if segmented(ui, p, &mut idx, &["PDF", "PDF (OCR)", "PNG"], hop) {
                app.scan_format = idx;
                app.hop(S_FORMAT);
            }
        });

        // 5: Save as
        self.form_row(ui, p, S_SAVE_AS, t.scan_labels[5], |app, ui, _| {
            let w = (ui.available_width() - 36.0).max(110.0);
            let resp = ui.add_sized(
                [w, 24.0],
                egui::TextEdit::singleline(&mut app.save_as).font(egui::TextStyle::Monospace),
            );
            app.text_focused |= resp.has_focus();
            if subtle_btn(ui, p, "…").clicked()
                && let Some(folder) = rfd::FileDialog::new().pick_folder()
            {
                let stem = name(&app.save_as).to_string();
                let stem = if stem.is_empty() {
                    format!("scan-{}", timestamp())
                } else {
                    stem
                };
                app.save_as = folder.join(stem).to_string_lossy().into_owned();
                app.hop(S_SAVE_AS);
            }
        });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(8.0);

        let busy = self.busy.is_some();
        let kept = self.scans.iter().filter(|p| p.keep).count();
        let all = self.scans.len();
        let save_label = match kept {
            _ if kept < all => fill(t.save_some, &[("n", &kept), ("all", &all)]),
            1 => t.save_one.into(),
            n => fill(t.save_many, &[("n", &n)]),
        };
        let copy_label = match kept {
            0 if self.scans.is_empty() => t.copy_scan.into(),
            1 => t.copy_one.into(),
            n => fill(t.copy_many, &[("n", &n)]),
        };

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            let scan_resp = primary_btn(ui, p, t.btn_scan, !busy);
            if scan_resp.hovered() {
                self.focused_row = S_SCAN;
            }
            if scan_resp.clicked() {
                self.scan_page(false, None);
            }

            let save_resp = accent_outline_btn(ui, p, &save_label, !busy && kept > 0);
            if save_resp.hovered() {
                self.focused_row = S_SAVE;
            }
            if save_resp.clicked() {
                self.save_scans();
            }

            let copy_resp = accent_outline_btn(ui, p, &copy_label, !busy);
            if copy_resp.hovered() {
                self.focused_row = S_COPY;
            }
            if copy_resp.clicked() {
                self.copy();
            }

            if !self.scans.is_empty() {
                let disc_resp = subtle_btn(ui, p, t.btn_discard);
                if disc_resp.hovered() {
                    self.focused_row = S_DISCARD;
                }
                if disc_resp.clicked() {
                    self.discard_scans();
                }
            } else if !self.discarded.is_empty() {
                let undo_resp = accent_outline_btn(ui, p, &format!("← {}", t.loaded_last), true);
                if undo_resp.clicked() {
                    self.undo_discard();
                }
            }
        });
    }

    fn draw_settings_form(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = t();

        // 0: Language
        self.form_row(ui, p, C_LANG, t.settings_labels[0], |app, ui, hop| {
            let total = i18n::ALL.len() + 1;
            let cur = if app.lang.is_empty() {
                0
            } else {
                i18n::find(&app.lang).map_or(0, |i| i + 1)
            };
            let sys_name = i18n::NAMES[i18n::find(&system_language()).unwrap_or(0)];
            let sys_label = fill(t.system, &[("lang", &sys_name)]);
            let cur_label = if cur == 0 {
                sys_label.clone()
            } else {
                i18n::NAMES[cur - 1].to_string()
            };
            let mut next_idx = cur;
            if stepper_arrow(ui, p, "‹", true).clicked() {
                next_idx = (cur + total - 1) % total;
            }
            let w = (ui.available_width() - 36.0).max(140.0);
            egui::ComboBox::from_id_salt("lang_combo")
                .width(w)
                .selected_text(RichText::new(cur_label).color(if hop { p.yellow } else { p.text }))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut next_idx, 0, &sys_label);
                    for (i, name) in i18n::NAMES.iter().enumerate() {
                        ui.selectable_value(&mut next_idx, i + 1, *name);
                    }
                });
            if stepper_arrow(ui, p, "›", true).clicked() {
                next_idx = (cur + 1) % total;
            }
            if next_idx != cur {
                app.lang = if next_idx == 0 {
                    String::new()
                } else {
                    i18n::ALL[next_idx - 1].code.to_string()
                };
                app.apply_lang_and_demo();
                app.save_settings();
                app.hop(C_LANG);
            }
        });

        // 1: Theme (strip "(terminal)" note from Auto in desktop GUI)
        self.form_row(ui, p, C_THEME, t.settings_labels[1], |app, ui, hop| {
            let auto_label = t.themes[0]
                .split(['(', '（'])
                .next()
                .unwrap_or(t.themes[0])
                .trim();
            let mut idx = app.theme;
            if segmented(ui, p, &mut idx, &[auto_label, t.themes[1], t.themes[2]], hop) {
                app.theme = idx;
                app.save_settings();
                app.hop(C_THEME);
            }
        });

        // 2: Volume
        self.form_row(ui, p, C_VOLUME, t.settings_labels[2], |app, ui, hop| {
            let mut vol = sound::volume();
            let prev = vol;
            if speaker_btn(ui, p, vol == 0).clicked() {
                vol = if vol == 0 { sound::DEFAULT_VOLUME } else { 0 };
            }
            ui.add(
                egui::Slider::new(&mut vol, 0..=100)
                    .step_by(10.0)
                    .show_value(false),
            );
            let txt = if vol == 0 {
                t.muted.to_string()
            } else {
                format!("{vol}%")
            };
            ui.label(
                RichText::new(txt)
                    .color(if hop { p.yellow } else { p.muted })
                    .monospace()
                    .size(12.0),
            );
            if vol != prev {
                sound::set_volume(vol);
                app.save_settings();
                app.hop(C_VOLUME);
            }
        });

        // 3: Mascot
        self.form_row(ui, p, C_MASCOT, t.settings_labels[3], |app, ui, hop| {
            let mut idx = usize::from(!app.mascot);
            if segmented(ui, p, &mut idx, &t.on_off, hop) {
                app.mascot = idx == 0;
                app.save_settings();
                app.hop(C_MASCOT);
            }
        });

        // 5: Scan folder
        self.form_row(ui, p, C_FOLDER, t.settings_labels[5], |app, ui, _| {
            let w = (ui.available_width() - 36.0).max(120.0);
            let resp = ui.add_sized(
                [w, 24.0],
                egui::TextEdit::singleline(&mut app.scan_folder)
                    .hint_text(RichText::new(default_scan_folder()).color(p.muted))
                    .font(egui::TextStyle::Monospace),
            );
            app.text_focused |= resp.has_focus();
            if resp.lost_focus() {
                app.save_as = default_scan_name(&app.scan_folder);
                app.save_settings();
            }
            if subtle_btn(ui, p, "…").clicked()
                && let Some(folder) = rfd::FileDialog::new().pick_folder()
            {
                app.scan_folder = folder.to_string_lossy().into_owned();
                app.save_as = default_scan_name(&app.scan_folder);
                app.save_settings();
                app.hop(C_FOLDER);
            }
        });
    }

    fn draw_stage(&mut self, ui: &mut egui::Ui, p: Palette) {
        let printer_state = if self.tab == Tab::Print {
            self.printer_state().cloned()
        } else {
            None
        };
        let inks = printer_state.as_ref().map_or(&[][..], |s| s.ink.as_slice());
        let ink_source = printer_state
            .as_ref()
            .map_or(InkSource::Unknown, |s| s.ink_source);
        let tick = self.started.elapsed().as_millis();

        let (text, work, progress_bar) = match &self.busy {
            Some((msg, _)) => (
                format!("{msg}…"),
                if self.scanning {
                    Work::Scan
                } else {
                    Work::Busy
                },
                None,
            ),
            None if self.editing.is_some() => (format!("{}…", t().editing_page), Work::Busy, None),
            None if self.viewing.is_some()
                && self.tab == Tab::Print
                && self.status.is_empty() =>
            {
                (format!("{}…", t().drawing_preview), Work::Busy, None)
            }
            None if !self.printing.is_empty()
                && !matches!(self.modal, Modal::Flip { job: None, .. }) =>
            {
                let (done, total) = self
                    .printing
                    .iter()
                    .fold((0, 0), |(d, tot), (_, sides, n)| (d + n, tot + sides));
                let total = total.max(1);
                let head = fill(
                    t().printing_page,
                    &[("i", &(done + 1).min(total)), ("n", &total)],
                );
                let mut extra_lines = Vec::new();
                if !self.status.is_empty() && !matches!(self.modal, Modal::Flip { job: None, .. }) {
                    extra_lines.push(self.status.clone());
                }
                if let Some(c) = self.complaint() {
                    extra_lines.push(c);
                }
                let extra_text = extra_lines.join("\n");
                (
                    extra_text,
                    Work::None,
                    Some((head, done as f32 / total as f32)),
                )
            }
            None if matches!(self.modal, Modal::Flip { job: Some(_), .. }) => {
                (self.status.clone(), Work::None, None)
            }
            None if matches!(self.modal, Modal::Flip { job: None, .. }) => {
                (t().flip_time.into(), Work::None, None)
            }
            None if self.status.is_empty() => {
                (self.complaint().unwrap_or_else(|| self.chat()), Work::None, None)
            }
            None => (self.status.clone(), Work::None, None),
        };

        let is_error = text.starts_with(t().error);
        let is_worried = !is_error && self.complaint().is_some_and(|c| c == text);
        let edge = if is_error {
            p.red
        } else if is_worried {
            p.yellow
        } else {
            p.accent
        };

        let avail_w = ui.available_width();
        let mascot_w = if self.mascot { 106.0 } else { 0.0 };
        let tanks_w = if !inks.is_empty() && avail_w >= mascot_w + 170.0 + inks.len() as f32 * 22.0
        {
            (inks.len() as f32 * 22.0 + 4.0).max(74.0)
        } else {
            0.0
        };
        let gaps = (if mascot_w > 0.0 { 10.0 } else { 0.0 })
            + (if tanks_w > 0.0 { 10.0 } else { 0.0 });
        let bubble_w = (avail_w - mascot_w - tanks_w - gaps).max(140.0);
        let stage_h = ui.available_height().max(96.0);

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;

            if self.mascot {
                let (rows, pad) = pet::printer_grid(self.act, work, self.mood(), tick);
                draw_pixel_grid(ui, &rows, pad, 4.6, Vec2::new(106.0, stage_h.min(100.0)));
            }

            // Speech bubble with vector left tail pointing at the mascot
            ui.allocate_ui_with_layout(
                Vec2::new(bubble_w, stage_h),
                Layout::top_down(Align::LEFT),
                |ui| {
                    ui.add_space(2.0);
                    let bubble_resp = Frame::new()
                        .fill(p.panel)
                        .stroke(Stroke::new(1.25_f32, edge))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(11, 8))
                        .show(ui, |ui| {
                            ui.set_width((bubble_w - 24.0).max(100.0));
                            ui.set_min_height(38.0);
                            if let Some((head, frac)) = &progress_bar {
                                ui.label(RichText::new(head).color(p.text).strong().size(13.0));
                                ui.add_space(4.0);
                                let (bar_rect, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 8.0),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    bar_rect,
                                    CornerRadius::same(4),
                                    p.control,
                                );
                                let fill_w = (bar_rect.width() * frac.clamp(0.04, 1.0)).max(6.0);
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        bar_rect.min,
                                        Vec2::new(fill_w, bar_rect.height()),
                                    ),
                                    CornerRadius::same(4),
                                    p.accent,
                                );
                                if !text.is_empty() {
                                    let compact = text
                                        .lines()
                                        .map(str::trim)
                                        .filter(|l| !l.is_empty())
                                        .collect::<Vec<_>>()
                                        .join(" · ");
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(compact).color(p.muted).size(11.5));
                                }
                            } else {
                                let text_color = if is_error {
                                    p.red
                                } else if is_worried {
                                    p.yellow
                                } else {
                                    p.text
                                };
                                ui.label(RichText::new(&text).color(text_color).size(13.0));
                            }
                        })
                        .response;

                    if self.mascot {
                        let r = bubble_resp.rect;
                        let cy = (r.top() + 26.0).min(r.center().y);
                        let tip = Pos2::new(r.left() - 7.0, cy);
                        let top = Pos2::new(r.left() + 1.0, cy - 6.0);
                        let bot = Pos2::new(r.left() + 1.0, cy + 6.0);
                        let painter = ui.painter();
                        painter.add(egui::Shape::convex_polygon(
                            vec![tip, top, bot],
                            p.panel,
                            Stroke::NONE,
                        ));
                        painter.line_segment([tip, Pos2::new(r.left(), cy - 6.0)], Stroke::new(1.25_f32, edge));
                        painter.line_segment([tip, Pos2::new(r.left(), cy + 6.0)], Stroke::new(1.25_f32, edge));
                    }
                    if bubble_resp.clicked() && !self.status.is_empty() {
                        self.status.clear();
                    }
                },
            );

            if tanks_w > 0.0 {
                draw_ink_tanks(ui, p, inks, ink_source);
            }
        });
    }

    fn draw_print_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette) {
        let files = split_files(&self.file);
        let multi = files.len() > 1;
        let header_title = match &self.view {
            Some((k, Ok(v))) if multi => format!("{}: {} · {}", t().preview, name(&k.file), v.title),
            Some((_, Ok(v))) => format!("{}: {}", t().preview, v.title),
            _ => t().preview.into(),
        };
        let (page_idx, page_count) = match &self.view {
            Some((_, Ok(v))) => (v.idx, v.count),
            _ => (0, 0),
        };

        // Preview header bar
        ui.horizontal(|ui| {
            ui.label(RichText::new(header_title).color(p.accent).strong().size(13.5));
            if self.viewing.is_some() {
                ui.add_space(6.0);
                ui.label(RichText::new(format!("({}…)", t().rendering)).color(p.muted).size(11.5));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if page_count > 1 || multi {
                    if stepper_arrow(ui, p, "›", true).clicked() {
                        self.step_preview(true);
                    }
                    if page_count > 0 {
                        ui.label(
                            RichText::new(format!("{} / {}", page_idx + 1, page_count))
                                .color(p.muted)
                                .monospace()
                                .size(12.0),
                        );
                    }
                    if stepper_arrow(ui, p, "‹", true).clicked() {
                        self.step_preview(false);
                    }
                }
                if multi {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("{}/{}", self.view_file + 1, files.len()))
                            .color(p.accent)
                            .monospace()
                            .size(11.5),
                    );
                }
            });
        });
        ui.add_space(6.0);

        let well_size = ui.available_size();
        Frame::new()
            .fill(p.panel_alt)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                let inner = (well_size - Vec2::new(26.0, 26.0)).max(Vec2::splat(120.0));
                ui.set_min_size(inner);
                ui.set_max_size(inner);
                if self.file.trim().is_empty() {
                    let resp = ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space((ui.available_height() * 0.30).max(20.0));
                            draw_empty_doc_art(ui, p, false);
                            ui.add_space(10.0);
                            ui.label(RichText::new(t().feed_me).color(p.text).strong().size(15.0));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(t().other_files)
                                    .color(p.muted)
                                    .size(12.5),
                            );
                            ui.add_space(12.0);
                            if accent_outline_btn(ui, p, t().k_select, true).clicked() {
                                let picked = pick_files();
                                if !picked.is_empty() {
                                    self.file = picked.join("; ");
                                    self.view_file = 0;
                                    self.view_page = 0;
                                    self.hop(R_FILE);
                                }
                            }
                        });
                    });
                    if resp.response.hovered() {
                        self.focused_row = R_FILE;
                    }
                    return;
                }
                match &mut self.view {
                    Some((_, Ok(v))) => {
                        let tex = v.texture.get_or_insert_with(|| {
                            ctx.load_texture("print_preview", v.image.clone(), TextureOptions::LINEAR)
                        });
                        draw_fitted_page(ui, p, tex, false);
                    }
                    Some((_, Err(e))) => {
                        ui.vertical_centered(|ui| {
                            ui.add_space((ui.available_height() * 0.36).max(20.0));
                            ui.label(RichText::new(e.as_str()).color(p.muted).size(13.0));
                        });
                    }
                    None => {
                        ui.vertical_centered(|ui| {
                            ui.add_space((ui.available_height() * 0.36).max(20.0));
                            ui.label(RichText::new(format!("{}…", t().rendering)).color(p.muted).size(13.0));
                        });
                    }
                }
            });
    }

    fn draw_scan_preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette) {
        let t = t();
        let total = self.scans.len();
        let title = if let Some(page) = self.scans.get(self.cur) {
            format!(
                "{}{}",
                fill(t.preview_scan, &[("n", &(self.cur + 1)), ("all", &total)]),
                if page.keep { "" } else { t.not_saved }
            )
        } else {
            t.preview.into()
        };

        // Preview header
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).color(p.accent).strong().size(13.5));
            if self.editing.is_some() {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("({}…)", t.editing_page))
                        .color(p.muted)
                        .size(11.5),
                );
            }
            if total > 1 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if stepper_arrow(ui, p, "›", true).clicked() {
                        self.cur = (self.cur + 1) % total;
                    }
                    ui.label(
                        RichText::new(format!("{} / {total}", self.cur + 1))
                            .color(p.muted)
                            .monospace()
                            .size(12.0),
                    );
                    if stepper_arrow(ui, p, "‹", true).clicked() {
                        self.cur = (self.cur + total - 1) % total;
                    }
                });
            }
        });
        ui.add_space(6.0);

        let toolbar_h = if total > 0 { 76.0 } else { 0.0 };
        let well_h = (ui.available_height() - toolbar_h).max(160.0);
        let well_w = ui.available_width();

        Frame::new()
            .fill(p.panel_alt)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                let inner_w = (well_w - 22.0).max(120.0);
                let inner_h = (well_h - 22.0).max(120.0);
                ui.set_min_size(Vec2::new(inner_w, inner_h));
                ui.set_max_width(inner_w);
                ui.set_max_height(inner_h);
                if let Some(page) = self.scans.get_mut(self.cur) {
                    let key = format!("scan_preview_{}_{}_{}", page.orig, page.rot, page.filter);
                    let tex = page
                        .texture
                        .get_or_insert_with(|| ctx.load_texture(key, page.image.clone(), TextureOptions::LINEAR));
                    draw_fitted_page(ui, p, tex, !page.keep);
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.30).max(20.0));
                        draw_empty_doc_art(ui, p, true);
                        ui.add_space(10.0);
                        ui.label(RichText::new(t.pages_here).color(p.text).size(14.0));
                        if !self.discarded.is_empty() {
                            ui.add_space(10.0);
                            if accent_outline_btn(ui, p, &format!("← {} (u)", t.loaded_last), true)
                                .clicked()
                            {
                                self.undo_discard();
                            }
                        }
                    });
                }
            });

        if total > 0 {
            ui.add_space(8.0);
            // Page pills strip
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for i in 0..total {
                    let selected = self.cur == i;
                    let kept = self.scans[i].keep;
                    let text = if kept {
                        format!("{}", i + 1)
                    } else {
                        format!("({})", i + 1)
                    };
                    let btn = egui::Button::new(
                        RichText::new(text)
                            .color(if selected {
                                Color32::WHITE
                            } else if kept {
                                p.text
                            } else {
                                p.muted
                            })
                            .monospace()
                            .size(12.0),
                    )
                    .fill(if selected { p.accent } else { p.control })
                    .corner_radius(CornerRadius::same(5));
                    if ui.add_sized([28.0, 22.0], btn).clicked() {
                        self.cur = i;
                        self.focused_row = S_PAGE;
                    }
                }
            });
            ui.add_space(6.0);

            // Editing toolbar (rotate, filter, keep, reorder, retake, delete)
            let cur = self.cur;
            let cur_rot = self.scans[cur].rot;
            let cur_filter = self.scans[cur].filter;
            let mut cur_keep = self.scans[cur].keep;

            let resp = ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                if subtle_btn(ui, p, "↺").on_hover_text(t.k_rotate).clicked() {
                    self.scans[cur].rot = (cur_rot + 270) % 360;
                    self.edit_current_scan();
                    self.hop(S_PAGE);
                }
                if subtle_btn(ui, p, "↻").on_hover_text(t.k_rotate).clicked() {
                    self.scans[cur].rot = (cur_rot + 90) % 360;
                    self.edit_current_scan();
                    self.hop(S_PAGE);
                }
                if subtle_btn(ui, p, &format!("{}: {}", t.k_filter, t.filters[cur_filter]))
                    .clicked()
                {
                    self.scans[cur].filter = (cur_filter + 1) % FILTERS.len();
                    self.edit_current_scan();
                    self.hop(S_PAGE);
                }
                if ui.checkbox(&mut cur_keep, t.keep).changed() {
                    self.scans[cur].keep = cur_keep;
                    self.hop(S_PAGE);
                }
                if stepper_arrow(ui, p, "←", cur > 0).on_hover_text(t.k_move).clicked() {
                    self.scans.swap(cur, cur - 1);
                    self.cur -= 1;
                    self.hop(S_PAGE);
                }
                if stepper_arrow(ui, p, "→", cur + 1 < total)
                    .on_hover_text(t.k_move)
                    .clicked()
                {
                    self.scans.swap(cur, cur + 1);
                    self.cur += 1;
                    self.hop(S_PAGE);
                }
                if subtle_btn(ui, p, t.k_retake).clicked() && self.busy.is_none() {
                    let orig = self.scans[cur].orig.clone();
                    self.scan_page(false, Some(orig));
                }
                let del_btn = egui::Button::new(RichText::new(t.k_delete).color(p.red).size(12.5))
                    .fill(p.control)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .corner_radius(CornerRadius::same(6));
                if ui.add_sized([0.0, 26.0], del_btn).clicked() {
                    self.delete_current_scan();
                }
            });
            if resp.response.hovered() {
                self.focused_row = S_PAGE;
            }
        }
    }

    fn draw_config_panel(&mut self, ui: &mut egui::Ui, p: Palette) {
        let path = config_path()
            .map(|cp| tilde(&cp.to_string_lossy()))
            .unwrap_or_else(|| "%APPDATA%\\printertui\\config".into());
        ui.label(
            RichText::new(&path)
                .color(p.accent)
                .strong()
                .monospace()
                .size(13.0),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(fill(t().saved_to, &[("path", &path)]))
                .color(p.muted)
                .size(12.0),
        );
        ui.add_space(10.0);

        let text = if !self.config_cache.is_empty() {
            self.config_cache.clone()
        } else {
            self.config_text()
        };
        Frame::new()
            .fill(p.panel_alt)
            .stroke(Stroke::new(1.0_f32, p.border))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (k, v) in parse_config(&text) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(RichText::new(k).color(p.accent).monospace().size(12.0));
                            ui.label(RichText::new("=").color(p.muted).monospace().size(12.0));
                            ui.label(RichText::new(v).color(p.text).monospace().size(12.0));
                        });
                    }
                });
            });
    }

    fn draw_modal(&mut self, ctx: &egui::Context, p: Palette) {
        if matches!(self.modal, Modal::None | Modal::Flip { job: Some(_), .. }) {
            return;
        }
        // Dimmed backdrop
        let screen = ctx.screen_rect();
        let backdrop = egui::Area::new(egui::Id::new("modal_backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    if p.dark {
                        Color32::from_black_alpha(150)
                    } else {
                        Color32::from_black_alpha(80)
                    },
                );
            });
        let _ = backdrop;

        let title = match &self.modal {
            Modal::Pages(on) => {
                let n = on.iter().filter(|b| **b).count();
                fill(t().pages_title, &[("n", &n), ("all", &on.len())])
            }
            Modal::Queue(_) => t().queue_title.into(),
            Modal::AddPrinter { .. } => t().add_title.into(),
            Modal::Flip { .. } => t().duplex_title.into(),
            Modal::Doctor(_) => extra().doctor_title.into(),
            Modal::None => String::new(),
        };

        let mut close = false;
        let mut submit_back = false;
        let mut apply_pages: Option<String> = None;
        let mut cancel_job_id: Option<(String, String)> = None;
        let mut add_target: Option<(String, String)> = None;
        let mut recheck_doctor = false;
        let tick = self.started.elapsed().as_millis() as usize;
        let status_snapshot = self.status.clone();

        let modal_w = match &self.modal {
            Modal::Flip { .. } => 600.0,
            Modal::Queue(_) | Modal::AddPrinter { .. } | Modal::Doctor(_) => 540.0,
            Modal::Pages(_) => 440.0,
            Modal::None => 400.0,
        };
        egui::Window::new(RichText::new(title).color(p.accent).strong().size(14.5))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(modal_w)
            .max_width(modal_w)
            .frame(
                Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1.5_f32, p.accent))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(modal_w - 32.0);
                match &mut self.modal {
                Modal::Pages(on) => {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            egui::Grid::new("pages_grid")
                                .num_columns(3)
                                .spacing([16.0, 6.0])
                                .show(ui, |ui| {
                                    for (i, checked) in on.iter_mut().enumerate() {
                                        let label = fill(t().page_item, &[("n", &(i + 1))]);
                                        ui.checkbox(checked, label);
                                        if i % 3 == 2 {
                                            ui.end_row();
                                        }
                                    }
                                });
                        });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let all = on.iter().all(|b| *b);
                        if subtle_btn(ui, p, if all { t().none } else { t().all }).clicked() {
                            on.iter_mut().for_each(|b| *b = !all);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary_btn(ui, p, t().k_done, true).clicked() {
                                let picked: Vec<u32> = (1..)
                                    .zip(on.iter())
                                    .filter(|(_, b)| **b)
                                    .map(|(n, _)| n)
                                    .collect();
                                if !picked.is_empty() {
                                    apply_pages = Some(if picked.len() == on.len() {
                                        String::new()
                                    } else {
                                        join(&picked)
                                    });
                                }
                            }
                            if subtle_btn(ui, p, t().k_cancel).clicked() {
                                close = true;
                            }
                        });
                    });
                }
                Modal::Queue(jobs) => {
                    if jobs.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(18.0);
                            ui.label(RichText::new(t().no_jobs).color(p.muted).size(14.0));
                            ui.add_space(18.0);
                        });
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(280.0)
                            .show(ui, |ui| {
                                for (id, desc) in jobs.iter() {
                                    Frame::new()
                                        .fill(p.panel_alt)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .corner_radius(CornerRadius::same(7))
                                        .inner_margin(Margin::symmetric(10, 8))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(format!("#{id}"))
                                                        .color(p.accent)
                                                        .strong()
                                                        .monospace(),
                                                );
                                                ui.label(RichText::new(desc).color(p.text));
                                                ui.with_layout(
                                                    Layout::right_to_left(Align::Center),
                                                    |ui| {
                                                        let btn = egui::Button::new(
                                                            RichText::new(t().k_cancel)
                                                                .color(p.red)
                                                                .size(12.0),
                                                        )
                                                        .fill(p.control)
                                                        .corner_radius(CornerRadius::same(5));
                                                        if ui.add(btn).clicked() {
                                                            cancel_job_id =
                                                                Some((id.clone(), desc.clone()));
                                                        }
                                                    },
                                                );
                                            });
                                        });
                                    ui.add_space(6.0);
                                }
                            });
                    }
                    ui.add_space(8.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_btn(ui, p, t().k_done, true).clicked() {
                            close = true;
                        }
                    });
                }
                Modal::AddPrinter { found, address } => {
                    if !found.is_empty() {
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for (pname, uri) in found.iter() {
                                    Frame::new()
                                        .fill(p.panel_alt)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .corner_radius(CornerRadius::same(7))
                                        .inner_margin(Margin::symmetric(10, 8))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        RichText::new(pname).color(p.text).strong(),
                                                    );
                                                    ui.label(
                                                        RichText::new(uri)
                                                            .color(p.muted)
                                                            .monospace()
                                                            .size(11.5),
                                                    );
                                                });
                                                ui.with_layout(
                                                    Layout::right_to_left(Align::Center),
                                                    |ui| {
                                                        if primary_btn(ui, p, t().k_add, true)
                                                            .clicked()
                                                        {
                                                            add_target =
                                                                Some((pname.clone(), uri.clone()));
                                                        }
                                                    },
                                                );
                                            });
                                        });
                                    ui.add_space(6.0);
                                }
                            });
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                    }
                    ui.label(RichText::new(t().add_by_address).color(p.text).strong());
                    ui.label(RichText::new(t().address_eg).color(p.muted).size(12.0));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 96.0).max(160.0);
                        let resp = ui.add_sized(
                            [w, 28.0],
                            egui::TextEdit::singleline(address)
                                .hint_text("192.168.1.46")
                                .font(egui::TextStyle::Monospace),
                        );
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if (primary_btn(ui, p, t().k_add, !address.trim().is_empty()).clicked()
                            || enter)
                            && !address.trim().is_empty()
                        {
                            let raw = address.trim().to_string();
                            let uri = if raw.contains("://") {
                                raw.clone()
                            } else {
                                format!("ipp://{raw}/ipp/print")
                            };
                            let host = uri_host(&uri).map_or(raw, |(_, h)| h.to_string());
                            add_target = Some((queue_name(&host), uri));
                        }
                    });
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if subtle_btn(ui, p, t().k_cancel).clicked() {
                            close = true;
                        }
                    });
                }
                Modal::Flip { .. } => {
                    // Exact soul of PrinterTUI manual duplex: instructions on the left,
                    // live 15-frame animated pixel-art sheet flip on the right!
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(350.0, 200.0),
                            Layout::top_down(Align::LEFT),
                            |ui| {
                                ui.label(RichText::new(&status_snapshot).color(p.text).size(13.5));
                            },
                        );
                        ui.add_space(12.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(200.0, 200.0),
                            Layout::top_down(Align::Center),
                            |ui| {
                                let (grid, caption) = pet::flip_grid(tick / 350);
                                draw_pixel_grid(ui, &grid, 0, 6.0, Vec2::new(124.0, 136.0));
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(t().flip[caption])
                                        .color(p.accent)
                                        .strong()
                                        .size(12.5),
                                );
                            },
                        );
                    });
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if subtle_btn(ui, p, t().k_cancel).clicked() {
                            close = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary_btn(ui, p, t().btn_print, true).clicked()
                                || ui.input(|i| i.key_pressed(Key::Enter))
                            {
                                submit_back = true;
                            }
                        });
                    });
                }
                Modal::Doctor(report) => {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            for line in report.lines() {
                                if line.is_empty() {
                                    ui.add_space(6.0);
                                } else if let Some(rest) = line.strip_prefix("✓ ") {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                        let stroke = Stroke::new(1.8_f32, p.green);
                                        ui.painter().line_segment(
                                            [Pos2::new(r.left() + 2.0, r.center().y), Pos2::new(r.left() + 5.5, r.bottom() - 3.0)],
                                            stroke,
                                        );
                                        ui.painter().line_segment(
                                            [Pos2::new(r.left() + 5.5, r.bottom() - 3.0), Pos2::new(r.right() - 2.0, r.top() + 3.0)],
                                            stroke,
                                        );
                                        ui.label(RichText::new(rest).color(p.text).size(13.0));
                                    });
                                } else if let Some(rest) = line.strip_prefix("✗ ") {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                        let stroke = Stroke::new(1.8_f32, p.red);
                                        ui.painter().line_segment(
                                            [Pos2::new(r.left() + 3.0, r.top() + 3.0), Pos2::new(r.right() - 3.0, r.bottom() - 3.0)],
                                            stroke,
                                        );
                                        ui.painter().line_segment(
                                            [Pos2::new(r.right() - 3.0, r.top() + 3.0), Pos2::new(r.left() + 3.0, r.bottom() - 3.0)],
                                            stroke,
                                        );
                                        ui.label(RichText::new(rest).color(p.text).size(13.0));
                                    });
                                } else {
                                    ui.label(RichText::new(line).color(p.text).size(13.0));
                                }
                            }
                        });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if accent_outline_btn(ui, p, "↻", true).clicked() {
                            recheck_doctor = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary_btn(ui, p, t().k_done, true).clicked() {
                                close = true;
                            }
                        });
                    });
                }
                Modal::None => {}
            }
            });

        if close {
            if matches!(self.modal, Modal::Flip { .. }) {
                self.status = t().back_cancelled.into();
            }
            self.modal = Modal::None;
        }
        if submit_back {
            let res = self.back_side();
            self.show(res);
        }
        if let Some(spec) = apply_pages {
            self.pages = spec;
            self.view_page = 0;
            self.modal = Modal::None;
            self.hop(R_PAGES);
        }
        if let Some((id, desc)) = cancel_job_id {
            let result = if self.dummy {
                cancel_dummy_job(&mut self.dummy_jobs, &id)
            } else {
                cancel_job(&id)
            };
            self.status = match result {
                Ok(()) => fill(t().cancelled, &[("desc", &desc)]),
                Err(e) => failed(e),
            };
            if self.dummy
                && let Modal::Queue(ref mut jobs) = self.modal
            {
                *jobs = dummy_queue(&self.dummy_jobs);
            } else {
                self.open_queue();
            }
        }
        if let Some((pname, uri)) = add_target {
            self.add_network_printer(pname, uri);
        }
        if recheck_doctor {
            self.modal = Modal::None;
            self.open_doctor();
        }
    }
}

fn draw_pixel_grid(
    ui: &mut egui::Ui,
    rows: &[Vec<char>],
    pad_cols: usize,
    px: f32,
    box_size: Vec2,
) {
    let (rect, _) = ui.allocate_exact_size(box_size, Sense::hover());
    let painter = ui.painter();
    let origin = Pos2::new(
        (rect.left() + pad_cols as f32 * px).round(),
        rect.top().round(),
    );
    for (y, row) in rows.iter().enumerate() {
        for (x, &ch) in row.iter().enumerate() {
            let Some((r, g, b)) = pet::palette_rgb(ch) else {
                continue;
            };
            let min = Pos2::new(origin.x + x as f32 * px, origin.y + y as f32 * px);
            painter.rect_filled(
                Rect::from_min_size(min, Vec2::splat(px)),
                0.0,
                Color32::from_rgb(r, g, b),
            );
        }
    }
}

fn draw_mini_printer_badge(ui: &mut egui::Ui) {
    let (rows, _) = pet::printer_grid(Act::Idle, Work::None, Mood::Fine, 0);
    draw_pixel_grid(ui, &rows, 0, 1.3, Vec2::new(26.0, 14.0));
}

/// Crisp pixel-art page illustration for empty Print/Scan preview wells (zero font-glyph fallback).
fn draw_empty_doc_art(ui: &mut egui::Ui, p: Palette, scan_beam: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(48.0, 58.0), Sense::hover());
    let painter = ui.painter();
    let paper = if p.dark {
        Color32::from_rgb(48, 51, 60)
    } else {
        Color32::WHITE
    };
    painter.rect_filled(
        rect.translate(Vec2::new(2.0, 2.0)),
        CornerRadius::same(4),
        Color32::from_black_alpha(if p.dark { 65 } else { 22 }),
    );
    painter.rect_filled(rect, CornerRadius::same(4), paper);
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.25_f32, p.border_strong),
        StrokeKind::Inside,
    );
    // Blue title bar + muted text lines (echoing pet::PAPER)
    let x0 = rect.left() + 8.0;
    painter.rect_filled(
        Rect::from_min_size(Pos2::new(x0, rect.top() + 9.0), Vec2::new(22.0, 4.0)),
        CornerRadius::same(1),
        p.accent,
    );
    for (i, w) in [(0, 32.0), (1, 26.0), (2, 30.0), (3, 18.0)] {
        let y = rect.top() + 19.0 + i as f32 * 8.0;
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 3.0)),
            CornerRadius::same(1),
            p.border_strong,
        );
    }
    if scan_beam {
        let beam_y = rect.center().y + 3.0;
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(rect.left() - 3.0, beam_y - 1.5), Vec2::new(rect.width() + 6.0, 3.0)),
            CornerRadius::same(1),
            Color32::from_rgb(65, 220, 235),
        );
    }
}

/// Crisp vector speaker toggle button (never depends on system emoji fonts).
fn speaker_btn(ui: &mut egui::Ui, p: Palette, muted: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 24.0), Sense::click());
    let fill = if resp.hovered() {
        p.control_hover
    } else {
        p.control
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, if resp.hovered() { p.border_strong } else { p.border }),
        StrokeKind::Inside,
    );
    let fg = if muted { p.muted } else { p.accent };
    let cx = rect.left() + 7.0;
    let cy = rect.center().y;
    // Speaker cone
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(cx, cy), Vec2::new(3.5, 5.0)),
        0.0,
        fg,
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(cx + 1.5, cy - 2.5),
            Pos2::new(cx + 6.0, cy - 6.0),
            Pos2::new(cx + 6.0, cy + 6.0),
            Pos2::new(cx + 1.5, cy + 2.5),
        ],
        fg,
        Stroke::NONE,
    ));
    if muted {
        let s = Stroke::new(1.5_f32, p.red);
        painter.line_segment([Pos2::new(cx + 9.0, cy - 3.5), Pos2::new(cx + 14.5, cy + 3.5)], s);
        painter.line_segment([Pos2::new(cx + 14.5, cy - 3.5), Pos2::new(cx + 9.0, cy + 3.5)], s);
    } else {
        let s = Stroke::new(1.4_f32, fg);
        painter.line_segment([Pos2::new(cx + 9.0, cy - 2.5), Pos2::new(cx + 9.0, cy + 2.5)], s);
        painter.line_segment([Pos2::new(cx + 12.5, cy - 4.5), Pos2::new(cx + 12.5, cy + 4.5)], s);
    }
    resp
}

fn draw_ink_tanks(
    ui: &mut egui::Ui,
    p: Palette,
    inks: &[(u32, i32, bool)],
    source: InkSource,
) {
    let src = match source {
        InkSource::Ipp => "IPP",
        InkSource::CupsCache => "CUPS",
        InkSource::Demo => "DEMO",
        InkSource::Unknown => "?",
    };
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(
            RichText::new(format!("{} · {src}", t().ink))
                .color(p.muted)
                .size(11.0),
        );
        let col_w = 20.0;
        let tank_w = 15.0;
        let tank_h = 46.0;
        let total_w = inks.len() as f32 * col_w;
        let (area, _) = ui.allocate_exact_size(Vec2::new(total_w, tank_h + 16.0), Sense::hover());
        let painter = ui.painter();
        let frame_col = if p.dark {
            Color32::from_rgb(84, 88, 98)
        } else {
            Color32::from_rgb(120, 126, 140)
        };
        let well_col = if p.dark {
            Color32::from_rgb(20, 22, 26)
        } else {
            Color32::from_rgb(225, 228, 236)
        };
        for (i, &(rgb, level, low)) in inks.iter().enumerate() {
            let x0 = area.left() + i as f32 * col_w + (col_w - tank_w) * 0.5;
            let rect = Rect::from_min_size(Pos2::new(x0, area.top()), Vec2::new(tank_w, tank_h));
            painter.rect_filled(rect, CornerRadius::same(3), frame_col);
            let inner = rect.shrink(2.0);
            painter.rect_filled(inner, CornerRadius::same(1), well_col);
            if level > 0 {
                let frac = (level.clamp(0, 100) as f32) / 100.0;
                let h = (inner.height() * frac).max(2.0);
                let fill_rect =
                    Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - h), inner.max);
                let (r, g, b) = ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
                painter.rect_filled(fill_rect, 0.0, Color32::from_rgb(r, g, b));
                // Subtle gloss strip makes black ink tanks clearly legible on dark theme
                if r < 60 && g < 60 && b < 60 {
                    let sheen = Rect::from_min_size(
                        Pos2::new(fill_rect.left() + 1.5, fill_rect.top()),
                        Vec2::new(2.0, fill_rect.height()),
                    );
                    painter.rect_filled(sheen, 0.0, Color32::from_white_alpha(52));
                }
            }
            let txt = if level < 0 {
                "?".into()
            } else {
                format!("{level}")
            };
            painter.text(
                Pos2::new(rect.center().x, rect.bottom() + 9.0),
                egui::Align2::CENTER_CENTER,
                txt,
                egui::FontId::monospace(10.5),
                if low { p.red } else { p.muted },
            );
        }
    });
}

fn draw_fitted_page(ui: &mut egui::Ui, p: Palette, tex: &TextureHandle, dimmed: bool) {
    let avail = ui.available_size();
    let tex_size = tex.size_vec2();
    let scale = (avail.x / tex_size.x.max(1.0)).min(avail.y / tex_size.y.max(1.0));
    let draw_size = (tex_size * scale).max(Vec2::splat(16.0));
    let (rect, _) = ui.allocate_exact_size(avail, Sense::hover());
    let img_rect = Rect::from_center_size(rect.center(), draw_size);
    let painter = ui.painter();
    // Subtle paper drop shadow
    painter.rect_filled(
        img_rect.translate(Vec2::new(2.0, 3.0)),
        CornerRadius::same(2),
        Color32::from_black_alpha(if p.dark { 75 } else { 28 }),
    );
    let tint = if dimmed {
        Color32::from_rgba_unmultiplied(255, 255, 255, 110)
    } else {
        Color32::WHITE
    };
    painter.image(
        tex.id(),
        img_rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        tint,
    );
    painter.rect_stroke(
        img_rect,
        CornerRadius::same(1),
        Stroke::new(1.0_f32, if dimmed { p.red } else { p.border_strong }),
        StrokeKind::Outside,
    );
}

fn segmented(
    ui: &mut egui::Ui,
    p: Palette,
    current: &mut usize,
    options: &[&str],
    hop: bool,
) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, &label) in options.iter().enumerate() {
                    let on = *current == i;
                    let fg = if on {
                        if hop { p.yellow } else { Color32::WHITE }
                    } else {
                        p.muted
                    };
                    let btn = egui::Button::new(RichText::new(label).color(fg).size(12.5))
                        .fill(if on { p.accent } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(4));
                    if ui.add_sized([0.0, 22.0], btn).clicked() && !on {
                        *current = i;
                        changed = true;
                    }
                }
            });
        });
    changed
}

fn stepper_arrow(ui: &mut egui::Ui, p: Palette, glyph: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(glyph)
            .color(if enabled { p.accent } else { p.muted })
            .strong()
            .size(13.5),
    )
    .fill(p.control)
    .stroke(Stroke::new(1.0_f32, p.border))
    .corner_radius(CornerRadius::same(5));
    ui.add_enabled(enabled, |ui: &mut egui::Ui| ui.add_sized([24.0, 24.0], btn))
}

fn primary_btn(ui: &mut egui::Ui, p: Palette, label: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .color(Color32::WHITE)
            .strong()
            .size(13.5),
    )
    .fill(if enabled { p.accent } else { p.border })
    .stroke(Stroke::NONE)
    .corner_radius(CornerRadius::same(6));
    ui.add_enabled(enabled, |ui: &mut egui::Ui| {
        let resp = ui.add_sized([0.0, 30.0], btn);
        if resp.hovered() && enabled {
            ui.painter().rect_filled(
                resp.rect,
                CornerRadius::same(6),
                Color32::from_rgba_unmultiplied(
                    p.accent_hover.r(),
                    p.accent_hover.g(),
                    p.accent_hover.b(),
                    45,
                ),
            );
        }
        resp
    })
}

fn accent_outline_btn(ui: &mut egui::Ui, p: Palette, label: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(
        RichText::new(label)
            .color(if enabled { p.accent } else { p.muted })
            .strong()
            .size(13.0),
    )
    .fill(p.control)
    .stroke(Stroke::new(1.0_f32, if enabled { p.accent } else { p.border }))
    .corner_radius(CornerRadius::same(6));
    ui.add_enabled(enabled, |ui: &mut egui::Ui| ui.add_sized([0.0, 30.0], btn))
}

fn subtle_btn(ui: &mut egui::Ui, p: Palette, label: &str) -> egui::Response {
    let btn = egui::Button::new(RichText::new(label).color(p.text).size(12.5))
        .fill(p.control)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(6));
    ui.add_sized([0.0, 26.0], btn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_dummy_duplex_pads_odd_pages() {
        let pdf = "test/fisica_3paginas.pdf";
        let mut app = GuiApp::new_state(false, pdf.into(), true);
        app.duplex = true;
        let res = app.duplex_batch(vec![pdf.into()], 0, String::new(), true);
        assert!(res.is_ok());
        assert_eq!(app.dummy_jobs.len(), 1);
        assert_eq!(app.dummy_jobs[0].sides, 2);
        let (back_job, back_pages) = match &app.modal {
            Modal::Flip { back, .. } => (back.0.clone(), back.1),
            _ => panic!("Expected Modal::Flip"),
        };
        assert_eq!(back_pages, 2);
        assert_eq!(back_job.pages.as_deref(), Some("2,4"));
        assert!(app.back_side().is_ok());
        assert_eq!(app.dummy_jobs.len(), 2);
    }

    #[test]
    fn gui_config_roundtrip() {
        let mut app = GuiApp::new_state(false, String::new(), true);
        app.color = true;
        app.duplex = true;
        app.copies = 3;
        app.scale = SCALES.iter().position(|&s| s == 125).unwrap();
        app.quality = 2;
        app.theme = 2;
        app.mascot = false;
        app.lang = "es".into();
        let text = app.config_text();
        let mut restored = GuiApp::new_state(false, String::new(), true);
        for (k, v) in parse_config(&text) {
            restored.apply(k, v);
        }
        assert!(restored.color);
        assert!(restored.duplex);
        assert_eq!(restored.copies, 3);
        assert_eq!(SCALES[restored.scale], 125);
        assert_eq!(restored.quality, 2);
        assert_eq!(restored.theme, 2);
        assert!(!restored.mascot);
        assert_eq!(restored.lang, "es");
    }
}
