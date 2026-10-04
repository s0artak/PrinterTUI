//! Images in the terminal: the kitty graphics protocol (kitty, Ghostty) and Sixel (Windows
//! Terminal, foot, WezTerm...), for the scanned page and print previews.

use super::*;

impl App {
    /// Sends the scanned page preview to the terminal using Kitty graphics or Sixel protocol.
    pub(crate) fn sync_image(&mut self) {
        match self.graphics {
            Graphics::Kitty(tmux) => {
                let area = self.preview_area.get();
                let want = self.preview_png().map(|p| (p, area.width, area.height));
                if want.is_some() && want != self.sent && area.width > 0 {
                    let (png, c, r) = want.clone().unwrap_or_default();
                    // the image itself, not its path: over SSH the terminal runs on another computer;
                    // shrunk to the cells it fills, so it is quick to send
                    let (cell_w, cell_h) = sixel_cell();
                    if let Ok(data) = fit_png(&png, (c as usize * cell_w) as u32, (r as usize * cell_h) as u32) {
                        emit(&kitty_chunks(&format!("a=T,U=1,f=100,i={},q=2,c={c},r={r}", image_id()), &base64(&data), tmux));
                    }
                    self.sent = want;
                }
            }
            Graphics::Sixel => {
                let area = self.preview_area.get();
                // an image is drawn over everything: wait for the popup on it to close, which also
                // wipes the part it covered, then draw the image again
                if self.popup_area.get().intersects(area) {
                    self.sent = None;
                    return;
                }
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
                    let (cell_w, cell_h) = sixel_cell();
                    let max_w = (area.width as usize).saturating_sub(1) * cell_w;
                    let max_h = (area.height as usize) * cell_h;
                    if max_w > 0 && max_h > 0 && *w > 0 && *h > 0 {
                        let scale = (max_w as f32 / *w as f32).min(max_h as f32 / *h as f32);
                        let ow = ((*w as f32 * scale) as usize).max(10).min(max_w);
                        let oh = (((*h as f32 * scale) as usize).div_ceil(6) * 6).max(6).min(max_h / 6 * 6);
                        if ow > 0 && oh > 0 {
                            let small = downscale(*w, *h, px, ow, oh);
                            let sixel = sixel_encode(ow, oh, &small);
                            let mut buf = String::new();
                            clear(&mut buf);
                            let cols_used = ((ow as f32 / cell_w as f32).ceil() as u16).min(area.width);
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
}

pub(crate) fn emit(s: &str) {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(s.as_bytes()).and_then(|_| stdout.flush());
}

/// Image ids are global to the terminal, so derive ours from the pid (24 bits, sent as the placeholder's RGB color).
pub(crate) fn image_id() -> u32 {
    std::process::id() & 0xFF_FFFF | 1
}

/// Row numbers for kitty Unicode placeholders (the first 100 of kitty's rowcolumn-diacritics.txt).
pub(crate) const ROW_DIACRITICS: &str = "\u{0305}\u{030D}\u{030E}\u{0310}\u{0312}\u{033D}\u{033E}\u{033F}\u{0346}\u{034A}\u{034B}\u{034C}\u{0350}\u{0351}\u{0352}\u{0357}\u{035B}\u{0363}\u{0364}\u{0365}\u{0366}\u{0367}\u{0368}\u{0369}\u{036A}\u{036B}\u{036C}\u{036D}\u{036E}\u{036F}\u{0483}\u{0484}\u{0485}\u{0486}\u{0487}\u{0592}\u{0593}\u{0594}\u{0595}\u{0597}\u{0598}\u{0599}\u{059C}\u{059D}\u{059E}\u{059F}\u{05A0}\u{05A1}\u{05A8}\u{05A9}\u{05AB}\u{05AC}\u{05AF}\u{05C4}\u{0610}\u{0611}\u{0612}\u{0613}\u{0614}\u{0615}\u{0616}\u{0617}\u{0657}\u{0658}\u{0659}\u{065A}\u{065B}\u{065D}\u{065E}\u{06D6}\u{06D7}\u{06D8}\u{06D9}\u{06DA}\u{06DB}\u{06DC}\u{06DF}\u{06E0}\u{06E1}\u{06E2}\u{06E4}\u{06E7}\u{06E8}\u{06EB}\u{06EC}\u{0730}\u{0732}\u{0733}\u{0735}\u{0736}\u{073A}\u{073D}\u{073F}\u{0740}\u{0741}\u{0743}\u{0745}\u{0747}\u{0749}\u{074A}";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Graphics {
    None,
    Kitty(bool),
    Sixel,
}

/// Kitty graphics protocol support: kitty and Ghostty, directly or through tmux when
/// `allow-passthrough` is on. Returns Some(inside tmux).
pub(crate) fn kitty_graphics() -> Option<bool> {
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

/// The size of a character cell in pixels: what the terminal reports, else Windows
/// Terminal's fixed 10 x 20 (it scales images from that to its font, as the VT340 did), else a
/// small guess, so the image is at worst a little small, never spilling out of the preview.
pub(crate) fn sixel_cell() -> (usize, usize) {
    match ratatui::crossterm::terminal::window_size() {
        Ok(s) if s.width > 0 && s.height > 0 && s.columns > 0 && s.rows > 0 => ((s.width / s.columns).max(1) as usize, (s.height / s.rows).max(1) as usize),
        _ if std::env::var_os("WT_SESSION").is_some() => (10, 20),
        _ => (8, 16),
    }
}

pub(crate) fn detect_graphics() -> Graphics {
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
