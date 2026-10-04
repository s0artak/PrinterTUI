//! The Scan tab's work: scanning pages, editing them, saving them and copying them.

use super::*;

impl App {
    /// Page row keys; returns false for keys it does not use.
    pub(crate) fn page_key(&mut self, c: char, dd: bool) -> bool {
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
                forget_scan(&self.scans.remove(i).orig);
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
    pub(crate) fn edit_page(&mut self) {
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
                // a page deleted meanwhile has nothing left to edit
                Err(e) if app.scans.iter().any(|p| p.orig == orig) => app.status = failed(e),
                Err(_) => {}
            })
        }));
    }

    pub(crate) fn find_scanners(&mut self, full: bool) {
        self.spawn(t().searching_scanners, move || {
            let list = scanners(full);
            Box::new(move |app: &mut App| {
                app.scanner = list.iter().position(|(d, _)| *d == app.scanner_pref).unwrap_or(0);
                app.status = if list.is_empty() { fill(t().no_scanners, &[("hint", &t().hints()[1])]) } else { String::new() };
                app.scanners = Some(list);
            })
        });
    }

    pub(crate) fn scan_enter(&mut self) {
        match self.sel {
            SCANNER => self.find_scanners(true),
            SCAN => self.scan_page(false),
            SAVE => self.save_scans(),
            COPY => self.copy(),
            DISCARD => {
                self.scans.drain(..).for_each(|p| forget_scan(&p.orig));
                self.cur = 0;
                self.status = t().discarded.into();
            }
            _ => {}
        }
    }

    /// Scans a page; `then_copy` prints it right away (Copy with nothing scanned yet).
    pub(crate) fn scan_page(&mut self, then_copy: bool) {
        let Some((device, _)) = self.scanners.as_ref().and_then(|l| l.get(self.scanner)).cloned() else {
            return self.status = failed(t().no_scanner_selected);
        };
        let dir = scan_dir();
        let n = self.scans.len() + 1;
        // unique name: pages can be deleted and reordered, and old edits must not be reused
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let path = dir.join(format!("page-{stamp}")).to_string_lossy().into_owned();
        let (mode, dpi) = (SCAN_MODES[self.scan_mode], SCAN_DPI[self.scan_dpi]);
        self.scanning = true;
        sound::play(Sound::Scan);
        self.spawn(fill(t().scanning, &[("n", &n)]), move || {
            let res = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| scan(&device, mode, dpi, &path)).and_then(|_| thumbnail(&path));
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
    pub(crate) fn copy(&mut self) {
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
        let pdf = scan_dir().join("copy.pdf");
        let (dpi, scale, paper) = (SCAN_DPI[self.scan_dpi], SCALES[self.scale], PAPERS[self.paper]);
        let job = self.print_settings().job;
        self.send(t().printing_copy, move || {
            let res = images_to_pdf(&files, dpi, &[])
                .and_then(|data| std::fs::write(&pdf, data).map_err(|e| format!("{}: {e}", pdf.display())))
                .and_then(|_| printable(&pdf.to_string_lossy(), scale, paper))
                .and_then(|file| submit(&Job { file, ..job }));
            Box::new(move |app: &mut App| app.show(res.map(|id| fill(t().copy_sent, &[("id", &id)]))))
        });
    }

    pub(crate) fn save_scans(&mut self) {
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
                        // saved where they belong, so the scans in the temp folder can go
                        app.scans.drain(..).for_each(|p| forget_scan(&p.orig));
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
