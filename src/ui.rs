//! Drawing the screen: the form, the printer and its speech bubble, the preview and the popups.

use super::*;

pub(crate) fn draw(f: &mut Frame, app: &App) {
    if let (Some(bg), Some(fg)) = (theme().bg, theme().fg) {
        f.render_widget(Block::default().style(Style::new().bg(bg).fg(fg)), f.area());
    }
    let rows = app.rows();
    let [main, help] = Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(f.area());
    let [left, preview] = Layout::horizontal([Constraint::Min(40), Constraint::Percentage(45)]).areas(main);
    let [form, status] = Layout::vertical([Constraint::Length(rows as u16 + 2), Constraint::Min(3)]).areas(left);

    let lines: Vec<Line> = (0..rows).map(|i| form_row(app, label(app.tab, i), i)).collect();
    let tab = |name: &'static str, on: bool| {
        Span::styled(format!(" {name} "), if on { Style::new().fg(WHITE).bg(ACCENT).bold() } else { Style::new().fg(theme().dim) })
    };
    // same widths as the mouse expects: " PrinterTUI  " then " Print " and " Scan "
    let title = Line::from(vec![
        Span::styled(" PrinterTUI  ", Style::new().bold()),
        tab(t().tab_print, app.tab == Tab::Print),
        tab(t().tab_scan, app.tab == Tab::Scan),
        tab(t().tab_settings, app.tab == Tab::Settings),
        Span::raw(" "),
    ]);
    f.render_widget(Paragraph::new(lines).block(panel(title)), form);
    app.form_area.set(form);
    draw_preview(f, app, preview);
    draw_stage(f, app, status);

    let t = t();
    let keys: &[(&str, &str)] = match app.mode {
        Mode::Insert => &[("Esc", t.k_done), ("Enter", t.k_done)],
        Mode::Address(_) => &[("Enter", t.k_add), ("Esc", t.k_cancel)],
        _ if app.scanning => &[("Esc", t.k_cancel), ("q", t.k_quit)],
        _ if app.tab == Tab::Settings && app.sel == FOLDER => &[("j/k", t.k_move), ("i", t.k_edit_text), ("Tab", t.k_print_scan), ("q", t.k_quit)],
        _ if app.tab == Tab::Settings => &[("j/k", t.k_move), ("h/l", t.k_change), ("Tab", t.k_print_scan), ("q", t.k_quit)],
        _ if app.sel == PAGE && app.tab == Tab::Scan => {
            &[("H/L", t.k_page), ("</>", t.k_move), ("r/R", t.k_rotate), ("f", t.k_filter), ("x", t.k_keep), ("dd", t.k_delete)]
        }
        _ if app.tab == Tab::Scan && !app.scans.is_empty() => {
            &[("j/k", t.k_move), ("h/l", t.k_change), ("H/L", t.k_page), ("Enter", t.k_select), ("Tab", t.k_print_scan), ("q", t.k_quit)]
        }
        _ if app.sel == PAGES && app.tab == Tab::Print => &[("j/k", t.k_move), ("Enter", t.k_pick_pages), ("i", t.k_type_range), ("q", t.k_quit)],
        _ if app.tab == Tab::Print => &[
            ("j/k", t.k_move),
            ("h/l", t.k_change),
            ("H/L", t.k_preview_page),
            ("i", t.k_edit_text),
            ("Enter", t.k_select),
            ("Tab", t.k_print_scan),
            ("q", t.k_quit),
        ],
        _ => &[("j/k", t.k_move), ("h/l", t.k_change), ("i", t.k_edit_text), ("Enter", t.k_select), ("Tab", t.k_print_scan), ("q", t.k_quit)],
    };
    let chips: Vec<Span> = keys
        .iter()
        .flat_map(|(k, what)| {
            [
                Span::styled(format!(" {k} "), Style::new().fg(theme().chip.0).bg(theme().chip.1)),
                Span::styled(format!(" {what}  "), Style::new().fg(theme().dim)),
            ]
        })
        .collect();
    f.render_widget(Line::from(chips), help);
    let tick = app.started.elapsed().as_millis() as usize;

    match &app.mode {
        Mode::Main | Mode::Insert => {}
        Mode::Address(addr) => {
            let area = popup(f, 76, 4);
            let text =
                vec![Line::from(format!(" {}: {addr}_", t.address)), Line::from(format!(" {}", t.address_eg)).style(Style::new().add_modifier(Modifier::DIM))];
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
            let items: Vec<ListItem> = on
                .iter()
                .enumerate()
                .map(|(i, b)| ListItem::new(format!(" [{}] {}", if *b { "x" } else { " " }, fill(t.page_item, &[("n", &(i + 1))]))))
                .collect();
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

/// Columns a text takes on screen (Chinese characters take two).
pub(crate) fn width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Lines `text` takes wrapped at word boundaries into `cols` columns, as the bubble's paragraph wraps it.
pub(crate) fn wrapped_rows(text: &str, cols: usize) -> usize {
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
pub(crate) fn char_at(s: &str, col: usize) -> Option<char> {
    let mut x = 0;
    s.chars().find(|c| {
        x += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
        x > col
    })
}

/// A form row's label: Print tab rows past Scale and Scan tab buttons have none.
pub(crate) fn label(tab: Tab, row: usize) -> &'static str {
    match (tab, row) {
        (Tab::Print, PRINTER..=SCALE) => t().labels[row],
        (Tab::Scan, SCANNER..=SAVE_AS) => t().scan_labels[row],
        (Tab::Scan, PAGE) => t().scan_labels[5],
        (Tab::Settings, 0..SETTINGS_ROWS) => t().settings_labels[row],
        _ => "",
    }
}

/// Columns for the labels: the widest in this language, and a space.
pub(crate) fn label_width() -> usize {
    t().labels.iter().chain(&t().scan_labels).chain(&t().settings_labels).map(|l| width(l)).max().unwrap_or(0).max(11) + 1
}

/// The Settings tab's right panel: the settings file as it is on disk, updated as it changes.
pub(crate) fn draw_config(f: &mut Frame, _app: &App, area: ratatui::layout::Rect) {
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
pub(crate) fn value_col() -> u16 {
    1 + 3 + label_width() as u16
}

/// A rounded panel with a title in the accent color.
pub(crate) fn panel<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(theme().dim)).title(title.into().style(Style::new().fg(ACCENT).bold()))
}

/// A form row: " ▶ " on the selected one, the label, then the value: `< x >` as ‹ x ›, a
/// `[ button ]` as a pill. Both keep their width, so mouse clicks land on the same characters.
pub(crate) fn form_row<'a>(app: &App, label: &'a str, i: usize) -> Line<'a> {
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
        let v = if hop {
            Style::new().fg(theme().hop).bold()
        } else if on {
            Style::new().bold()
        } else {
            Style::new()
        };
        spans.extend([Span::styled("‹ ", arrows), Span::styled(inner.to_string(), v), Span::styled(" ›", arrows)]);
    } else {
        let cursor = if on && matches!(app.mode, Mode::Insert) { "_" } else { "" };
        spans.push(Span::styled(format!("{value}{cursor}"), if on { Style::new().bold() } else { Style::new() }));
    }
    Line::from(spans)
}

/// The printer and what it says (the status, or a word about the selected row), and the ink tanks.
pub(crate) fn draw_stage(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let inks = if app.tab != Tab::Print { &[][..] } else { app.printer_state().map_or(&[][..], |s| s.ink.as_slice()) };
    // the printer needs 22 columns and goes last on a narrow screen, after the ink tanks
    let pet_w = if app.mascot && area.width >= 22 + 24 { 22 } else { 0 };
    let tanks_w = if inks.is_empty() || area.width < pet_w + 24 + inks.len() as u16 * 4 + 1 { 0 } else { inks.len() as u16 * 4 + 1 };
    let [pet, bubble, tanks] = Layout::horizontal([Constraint::Length(pet_w), Constraint::Min(10), Constraint::Length(tanks_w)]).areas(area);
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
    let edge = if error {
        RED
    } else if worried {
        YELLOW
    } else {
        ACCENT
    };
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(edge)).padding(ratatui::widgets::Padding::horizontal(1));
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
pub(crate) fn draw_preview(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.tab == Tab::Settings {
        return draw_config(f, app, area);
    }
    let title = match (app.tab == Tab::Scan, app.scans.get(app.cur), &app.view) {
        (true, Some(p), _) => {
            format!(" {}{} ", fill(t().preview_scan, &[("n", &(app.cur + 1)), ("all", &app.scans.len())]), if p.keep { "" } else { t().not_saved })
        }
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
            let spans: Vec<Span> =
                (0..ow).map(|x| Span::styled("▀", Style::new().fg(gray(small[2 * y * ow + x])).bg(gray(small[(2 * y + 1) * ow + x])))).collect();
            Line::from(spans).centered()
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

thread_local! {
    /// The last popup drawn, copied into App::popup_area after each draw.
    pub(crate) static POPUP: Cell<ratatui::layout::Rect> = Cell::default();
}

pub(crate) fn popup(f: &mut Frame, width: u16, height: u16) -> ratatui::layout::Rect {
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
