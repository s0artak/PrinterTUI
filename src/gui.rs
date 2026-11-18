use eframe::egui::{self, Color32, RichText};
use printertui::*;

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([850.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PrinterTUI",
        options,
        Box::new(|cc| {
            style(&cc.egui_ctx);
            Ok(Box::new(PrinterApp::default()))
        }),
    )
}

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(10, 10, 10);
    v.window_fill = Color32::from_rgb(18, 18, 18);
    v.widgets.noninteractive.bg_fill = Color32::from_rgb(14, 14, 14);
    v.widgets.inactive.bg_fill = Color32::from_rgb(28, 28, 28);
    v.selection.bg_fill = Color32::from_rgb(74, 75, 79);
    ctx.set_visuals(v);
    let mut fonts = egui::FontDefinitions::default();
    if let Some(path) = google_sans_file()
        && let Ok(bytes) = std::fs::read(path)
    {
        fonts.font_data.insert(
            "google_sans".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "google_sans".into());
    }
    ctx.set_fonts(fonts);
}

fn google_sans_file() -> Option<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        dirs.push(std::path::PathBuf::from(windows).join("Fonts"));
    }
    dirs.extend([
        std::path::PathBuf::from("/System/Library/Fonts"),
        std::path::PathBuf::from("/Library/Fonts"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(home).join("Library/Fonts"));
    }
    dirs.into_iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    let name = name.to_ascii_lowercase();
                    name.starts_with("googlesans")
                        && (name.ends_with(".ttf") || name.ends_with(".otf"))
                })
        })
}

struct PrinterApp {
    printers: Vec<String>,
    printer: usize,
    file: String,
    page_range: String,
    copies: u32,
    duplex: bool,
    color: bool,
    tab: usize,
    status: String,
    queue: Vec<(String, String)>,
    scanners: Vec<(String, String)>,
    scan_folder: String,
    paper: usize,
    quality: usize,
    per_sheet: usize,
    reverse: bool,
    scanner: usize,
    source: usize,
    scan_mode: usize,
    dpi: usize,
    format: usize,
    scan_name: String,
    scans: Vec<ScanPage>,
    current_page: usize,
}
struct ScanPage {
    original: String,
    current: String,
    rotation: u16,
    filter: usize,
    keep: bool,
}
impl Default for PrinterApp {
    fn default() -> Self {
        let folder = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join("Documents");
        Self {
            printers: printers(),
            printer: 0,
            file: String::new(),
            page_range: String::new(),
            copies: 1,
            duplex: false,
            color: true,
            tab: 0,
            status: String::new(),
            queue: queue(),
            scanners: scanners(false),
            scan_folder: folder.display().to_string(),
            paper: 0,
            quality: 1,
            per_sheet: 0,
            reverse: false,
            scanner: 0,
            source: 0,
            scan_mode: 0,
            dpi: 1,
            format: 0,
            scan_name: "Scan".into(),
            scans: Vec::new(),
            current_page: 0,
        }
    }
}
impl eframe::App for PrinterApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::SidePanel::left("nav")
            .exact_width(220.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.heading(RichText::new("▣  PrinterTUI").size(19.0));
                ui.add_space(20.0);
                for (i, label) in ["Imprimir", "Escanear", "Cola", "Ajustes"]
                    .iter()
                    .enumerate()
                {
                    if ui
                        .selectable_label(
                            self.tab == i,
                            format!("{}   {label}", ["▤", "▧", "☷", "⚙"][i]),
                        )
                        .clicked()
                    {
                        self.tab = i;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.separator();
                    ui.label(RichText::new("Impresoras y escáneres").weak());
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(["Imprimir", "Escanear", "Cola", "Ajustes"][self.tab]);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳ Actualizar").clicked() {
                        self.printers = printers();
                        self.queue = queue();
                        self.scanners = scanners(false);
                    }
                });
            });
            ui.separator();
            match self.tab {
                0 => self.print_ui(ui),
                1 => self.scan_ui(ui),
                2 => {
                    for (id, desc) in &self.queue {
                        ui.horizontal(|ui| {
                            ui.label(format!("#{id}"));
                            ui.label(desc);
                            if ui.button("Cancelar").clicked() {
                                let _ = cancel_job(id);
                            }
                        });
                    }
                }
                _ => self.settings_ui(ui),
            }
            ui.add_space(14.0);
            if !self.status.is_empty() {
                ui.label(RichText::new(&self.status).color(Color32::from_rgb(170, 190, 170)));
            }
        });
    }
}
impl PrinterApp {
    fn print_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label("Impresora");
        egui::ComboBox::from_id_salt("printer")
            .selected_text(
                self.printers
                    .get(self.printer)
                    .cloned()
                    .unwrap_or_else(|| "No se encontraron impresoras".into()),
            )
            .show_ui(ui, |ui| {
                for (i, p) in self.printers.iter().enumerate() {
                    if ui.selectable_value(&mut self.printer, i, p).clicked() {}
                }
            });
        ui.add_space(16.0);
        ui.label("Documento");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.file);
            if ui.button("Elegir archivo…").clicked()
                && let Some(p) = rfd::FileDialog::new().pick_file()
            {
                self.file = p.display().to_string();
            }
        });
        ui.add_space(16.0);
        ui.label("Páginas (opcional)");
        ui.text_edit_singleline(&mut self.page_range);
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.copies).range(1..=999));
            ui.label("copias");
            ui.checkbox(&mut self.color, "Color");
            ui.checkbox(&mut self.duplex, "Doble cara");
            ui.checkbox(&mut self.reverse, "Orden inverso");
        });
        ui.horizontal(|ui| {
            ui.label("Papel");
            egui::ComboBox::from_id_salt("paper")
                .selected_text(PAPERS[self.paper])
                .show_ui(ui, |ui| {
                    for (i, v) in PAPERS.iter().enumerate() {
                        ui.selectable_value(&mut self.paper, i, *v);
                    }
                });
            ui.label("Páginas por hoja");
            egui::ComboBox::from_id_salt("sheets")
                .selected_text(PER_SHEET[self.per_sheet].to_string())
                .show_ui(ui, |ui| {
                    for (i, v) in PER_SHEET.iter().enumerate() {
                        ui.selectable_value(&mut self.per_sheet, i, v.to_string());
                    }
                });
            ui.label("Calidad");
            egui::ComboBox::from_id_salt("quality")
                .selected_text(["Borrador", "Normal", "Alta"][self.quality])
                .show_ui(ui, |ui| {
                    for (i, v) in ["Borrador", "Normal", "Alta"].iter().enumerate() {
                        ui.selectable_value(&mut self.quality, i, *v);
                    }
                });
        });
        ui.add_space(22.0);
        if ui
            .button(RichText::new("  Imprimir  ").size(16.0))
            .clicked()
        {
            self.submit();
        }
    }
    fn submit(&mut self) {
        let Some(printer) = self.printers.get(self.printer).cloned() else {
            self.status = "No hay impresoras disponibles".into();
            return;
        };
        if self.file.is_empty() {
            self.status = "Selecciona un documento".into();
            return;
        }
        let job = Job {
            printer,
            file: self.file.clone(),
            color: self.color,
            sides: self.duplex,
            paper: PAPERS[self.paper],
            pages: (!self.page_range.is_empty()).then(|| self.page_range.clone()),
            reverse: self.reverse,
            copies: self.copies.max(1),
            collate: true,
            per_sheet: PER_SHEET[self.per_sheet],
            quality: 3 + self.quality as u32,
        };
        self.status = match submit(&job) {
            Ok(s) => s,
            Err(e) => e,
        };
        self.queue = queue();
    }
    fn scan_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("scanner")
                .selected_text(
                    self.scanners
                        .get(self.scanner)
                        .map(|s| s.1.as_str())
                        .unwrap_or("Sin escáneres"),
                )
                .show_ui(ui, |ui| {
                    for (i, (_, name)) in self.scanners.iter().enumerate() {
                        ui.selectable_value(&mut self.scanner, i, name);
                    }
                });
            if ui.button("Buscar escáneres").clicked() {
                self.scanners = scanners(true);
                self.scanner = 0;
            }
            egui::ComboBox::from_id_salt("source")
                .selected_text(SCAN_SOURCES[self.source])
                .show_ui(ui, |ui| {
                    for (i, v) in SCAN_SOURCES.iter().enumerate() {
                        ui.selectable_value(&mut self.source, i, *v);
                    }
                });
            egui::ComboBox::from_id_salt("mode")
                .selected_text(SCAN_MODES[self.scan_mode])
                .show_ui(ui, |ui| {
                    for (i, v) in SCAN_MODES.iter().enumerate() {
                        ui.selectable_value(&mut self.scan_mode, i, *v);
                    }
                });
            egui::ComboBox::from_id_salt("dpi")
                .selected_text(format!("{} dpi", SCAN_DPI[self.dpi]))
                .show_ui(ui, |ui| {
                    for (i, v) in SCAN_DPI.iter().enumerate() {
                        ui.selectable_value(&mut self.dpi, i, format!("{v} dpi"));
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Destino");
            ui.text_edit_singleline(&mut self.scan_folder);
            if ui.button("Carpeta…").clicked()
                && let Some(p) = rfd::FileDialog::new().pick_folder()
            {
                self.scan_folder = p.display().to_string();
            }
            ui.label("Nombre");
            ui.text_edit_singleline(&mut self.scan_name);
            egui::ComboBox::from_id_salt("format")
                .selected_text(SCAN_FORMATS[self.format])
                .show_ui(ui, |ui| {
                    for (i, v) in SCAN_FORMATS.iter().enumerate() {
                        ui.selectable_value(&mut self.format, i, *v);
                    }
                });
            if ui.button("Escanear página").clicked() {
                self.capture(false);
            }
            if SCAN_SOURCES[self.source] == "Feeder" && ui.button("Escanear alimentador").clicked()
            {
                self.capture(true);
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.vertical(|ui| { ui.heading(format!("Páginas ({})",self.scans.len())); for (i,p) in self.scans.iter_mut().enumerate(){ui.horizontal(|ui|{if ui.selectable_label(self.current_page==i,format!("Página {}",i+1)).clicked(){self.current_page=i;}ui.checkbox(&mut p.keep,"");});} });
            ui.separator();
            if let Some(p)=self.scans.get_mut(self.current_page){
                ui.vertical(|ui| {ui.label(p.current.clone());ui.horizontal(|ui|{if ui.button("Girar ↻").clicked(){p.rotation=(p.rotation+90)%360;refresh_page(p);}
                    if ui.button("Girar ↺").clicked(){p.rotation=(p.rotation+270)%360;refresh_page(p);}
                    if ui.button("Filtro").clicked(){p.filter=(p.filter+1)%4;refresh_page(p);}
                    if ui.button("Eliminar página").clicked(){p.keep=false;}});});
            } else {ui.label("Escanea una página para empezar. Puedes revisar, girar, filtrar y descartar páginas antes de guardar.");}
        });
        if !self.scans.is_empty() {
            ui.horizontal(|ui| {
                if ui.button("Guardar páginas seleccionadas").clicked() {
                    self.save_scans();
                }
                if ui.button("Descartar todas").clicked() {
                    let files: Vec<_> = self.scans.iter().map(|p| p.original.clone()).collect();
                    clean_scan_files(&files);
                    self.scans.clear();
                }
            });
        }
    }
    fn capture(&mut self, stack: bool) {
        let Some((device, _)) = self.scanners.get(self.scanner).cloned() else {
            self.status = "No se encontró un escáner".into();
            return;
        };
        let root = std::path::PathBuf::from(&self.scan_folder).join(".printertui-scan");
        let _ = std::fs::create_dir_all(&root);
        let base = root.join(format!(
            "page-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        let stem = base.to_string_lossy().to_string();
        self.status = "Escaneando…".into();
        match scan_pages(
            &device,
            SCAN_MODES[self.scan_mode],
            SCAN_DPI[self.dpi],
            PAPERS[self.paper],
            SCAN_SOURCES[self.source],
            stack,
            |i| format!("{stem}-{}.png", i + 1),
        ) {
            Ok(paths) => {
                self.scans.extend(paths.into_iter().map(|path| ScanPage {
                    original: path.clone(),
                    current: path,
                    rotation: 0,
                    filter: 0,
                    keep: true,
                }));
                self.current_page = self.scans.len().saturating_sub(1);
                self.status = "Escaneo completado".into();
            }
            Err(e) => self.status = e,
        }
    }
    fn save_scans(&mut self) {
        let files: Vec<_> = self
            .scans
            .iter()
            .filter(|p| p.keep)
            .map(|p| p.current.clone())
            .collect();
        if files.is_empty() {
            self.status = "No hay páginas seleccionadas".into();
            return;
        }
        let out = std::path::PathBuf::from(&self.scan_folder)
            .join(&self.scan_name)
            .display()
            .to_string();
        self.status = match save_scans(
            &files,
            &out,
            SCAN_FORMATS[self.format],
            SCAN_DPI[self.dpi],
            &system_language(),
        ) {
            Ok(paths) => format!("Guardado: {}", paths.join(", ")),
            Err(e) => e,
        };
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Preferencias");
        ui.label("Carpeta predeterminada para guardar escaneos");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.scan_folder);
            if ui.button("Elegir…").clicked()
                && let Some(p) = rfd::FileDialog::new().pick_folder()
            {
                self.scan_folder = p.display().to_string();
            }
        });
        ui.label("Google Sans se usa si el sistema la tiene instalada; de lo contrario se aplica la fuente sans-serif del sistema.");
    }
}
fn refresh_page(page: &mut ScanPage) {
    if let Ok((path, _)) = edit_page(&page.original, page.rotation, page.filter) {
        page.current = path;
    }
}
