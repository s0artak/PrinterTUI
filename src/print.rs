//! The Print tab's work: the preview, sending the files to the printer (in the background), manual
//! double-sided printing, and keeping an eye on the print queue.

use super::*;

impl App {
    /// Renders the Print tab preview in the background whenever what it should show changes.
    pub(crate) fn sync_view(&mut self) {
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
        self.viewing = Some((
            key,
            task(move || {
                let res = render_view(&k);
                Box::new(move |app: &mut App| app.view = Some((k, res)))
            }),
        ));
    }

    /// Opens the page selector for the first file, pre-checking the current range.
    pub(crate) fn open_pages(&mut self) {
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

    /// The Print tab's settings for a print, copied when it starts, as it is sent in the background.
    pub(crate) fn print_settings(&self) -> PrintSettings {
        PrintSettings {
            job: Job {
                printer: self.printers.get(self.printer).cloned().unwrap_or_default(),
                file: String::new(),
                color: self.color,
                paper: PAPERS[self.paper],
                pages: None,
                reverse: false,
                copies: self.copies,
                collate: true,
                per_sheet: PER_SHEET[self.per_sheet],
            },
            pages: self.pages.clone(),
            duplex: self.duplex,
            reverse_back: self.reverse_back,
        }
    }

    pub(crate) fn files(&self) -> Result<Vec<String>, String> {
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

    /// Runs `work`, which sends a print, in the background; quitting waits for it.
    pub(crate) fn send(&mut self, msg: impl Into<String>, work: impl FnOnce() -> Done + Send + 'static) {
        self.spawn(msg, work);
        self.sending = true;
    }

    /// Converts the files to PDF and sends the jobs, in the background (drawing the pages for the
    /// printer takes a while on Windows).
    pub(crate) fn try_print(&mut self) {
        let files = match self.files() {
            Ok(f) => f,
            Err(e) => return self.show(Err(e)),
        };
        let (scale, paper) = (SCALES[self.scale], PAPERS[self.paper]);
        let print = self.print_settings();
        self.send(t().preparing, move || {
            let res = files.iter().map(|f| printable(f, scale, paper)).collect::<Result<Vec<_>, _>>().and_then(|pdfs| print_pdfs(&print, pdfs));
            Box::new(move |app: &mut App| app.printed(res))
        });
    }

    /// Sends the back side the person turned over, then the next files.
    pub(crate) fn back_side(&mut self) {
        let Mode::Flip { back, files, next, print, .. } = std::mem::replace(&mut self.mode, Mode::Main) else { return };
        self.send(t().preparing, move || {
            let res = submit(&back).and_then(|id| duplex(&print, files, next, fill(t().back_sent, &[("id", &id)]) + "\n\n"));
            Box::new(move |app: &mut App| app.printed(res))
        });
    }

    /// A sent print's result: the status, and the duplex popup when the stack has to be turned.
    pub(crate) fn printed(&mut self, res: Result<(String, Option<Mode>), String>) {
        let res = res.map(|(status, flip)| {
            if let Some(flip) = flip {
                self.mode = flip;
            }
            status
        });
        self.show(res);
    }

    /// Every second while they show: checks whether the duplex front job is done, and refreshes
    /// the print queue popup, in the background (a shared printer can take a while to answer).
    pub(crate) fn poll_jobs(&mut self) {
        if self.polling.is_some() || self.last_poll.elapsed() < Duration::from_secs(1) {
            return;
        }
        let front = match &self.mode {
            Mode::Flip { job: Some(id), .. } => Some(id.clone()),
            _ => None,
        };
        let refresh = matches!(self.mode, Mode::Queue(..));
        if front.is_none() && !refresh {
            return;
        }
        self.last_poll = Instant::now();
        self.polling = Some(task(move || {
            let printed = front.filter(|id| !job_active(id));
            let jobs = refresh.then(queue);
            Box::new(move |app: &mut App| {
                if let Mode::Flip { job, steps, .. } = &mut app.mode
                    && printed.is_some()
                    && *job == printed
                {
                    *job = None;
                    app.status = std::mem::take(steps);
                }
                if let (Mode::Queue(list, _), Some(jobs)) = (&mut app.mode, jobs) {
                    *list = jobs;
                }
            })
        }));
    }
}

/// The Print tab's settings for one print.
#[derive(Clone)]
pub(crate) struct PrintSettings {
    /// Printer, color, paper, copies and pages per sheet; each file gets its own name and pages.
    pub(crate) job: Job,
    pub(crate) pages: String,
    pub(crate) duplex: bool,
    pub(crate) reverse_back: bool,
}

/// Sends the PDFs; with manual duplex, their front sides until one needs turning over.
/// Returns the status, and the Flip mode to wait in for that.
pub(crate) fn print_pdfs(print: &PrintSettings, pdfs: Vec<String>) -> Result<(String, Option<Mode>), String> {
    if print.duplex {
        return duplex(print, pdfs, 0, String::new());
    }
    let pages = print.pages.trim();
    let all = pages.is_empty() || pages == "all";
    // checked against each PDF: CUPS takes page-ranges beyond the last page without a word
    let sent: Result<Vec<String>, String> = pdfs
        .iter()
        .map(|p| {
            let range = if all { None } else { Some(join(&parse_ranges(pages, page_count(p).ok_or(t().unreadable_pdf)?)?)) };
            submit(&Job { file: p.clone(), pages: range, ..print.job.clone() })
        })
        .collect();
    Ok((sent?.join("\n"), None))
}

/// Prints the front of files[i..] until one needs flipping, then waits in Mode::Flip.
pub(crate) fn duplex(print: &PrintSettings, files: Vec<String>, mut i: usize, mut done: String) -> Result<(String, Option<Mode>), String> {
    while let Some(file) = files.get(i) {
        i += 1;
        let head = if files.len() > 1 { fill(t().file_of, &[("i", &i), ("n", &files.len()), ("name", &name(file))]) + "\n" } else { String::new() };
        let pdf = file.clone();
        let total = page_count(&pdf).ok_or(t().unreadable_pdf)?;
        // page-ranges picks pages before number-up groups them, so split whole sheet sides
        let pages = parse_ranges(&print.pages, total)?;
        let sides: Vec<&[u32]> = pages.chunks(print.job.per_sheet as usize).collect();
        let (front, back) = split_duplex(&sides);
        // Collated copies would leave a sheet without back side inside every copy when odd.
        let odd = front.len() > back.len();
        let (front, back) = (front.concat(), back.concat());
        let id = submit(&Job { file: pdf.clone(), pages: Some(join(&front)), collate: !odd, ..print.job.clone() })?;
        if back.is_empty() {
            done += &format!("{head}{}\n\n", fill(t().one_page, &[("id", &id)]));
            continue;
        }
        let aside = match (odd, print.job.copies) {
            (false, _) => String::new(),
            (true, 1) => t().aside_one.into(),
            (true, n) => fill(t().aside_many, &[("n", &n)]),
        };
        let sent = fill(t().front_sent, &[("id", &id)]);
        let steps = format!("{done}{head}{sent}\n\n{}", fill(t().flip_steps, &[("aside", &aside)]));
        let wait = format!("{done}{head}{sent}\n\n{}", t().printing_front);
        // unparsable lp output: "" is never listed as active, so the steps show right away
        let job = Some(job_id(&id).unwrap_or_default().to_string());
        let back = Job { file: pdf, pages: Some(join(&back)), reverse: print.reverse_back, collate: !odd, ..print.job.clone() };
        return Ok((wait, Some(Mode::Flip { job, steps, back, files, next: i, print: print.clone() })));
    }
    Ok((done.trim_end().to_string(), None))
}

/// The preview for a key: the file as it will print (a PDF, scaled), one sheet side of its range.
pub(crate) fn render_view(k: &ViewKey) -> Result<PrintView, String> {
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
