//! The mascot: the installer's pixel-art printer (the same art and colors as install.sh), drawn
//! with half blocks, reacting to what the app does.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::time::{Duration, Instant};

pub const ACCENT: Color = Color::Indexed(69);
pub const DIM: Color = Color::Indexed(245);
pub const RED: Color = Color::Indexed(203);
pub const YELLOW: Color = Color::Indexed(221);
pub const WHITE: Color = Color::Indexed(231);

// . empty  w paper  k ink  b blue  d dark body  g light body  s slot  G green  r red  c cyan
// L M the two lights
const PRINTER: [&str; 10] = [
    "......wwwwwwww......",
    "......wwwwwwww......",
    "..bbbbbbbbbbbbbbbb..",
    ".bbbbbbbbbbbbbbbbbb.",
    ".dggggggggggggggggd.",
    ".dggggggggggggLgMgd.",
    ".dggggggggggggggggd.",
    ".dddddddddddddddddd.",
    "..dssssssssssssssd..",
    "...dddddddddddddd...",
];
const PAPER: [&str; 10] = [
    "...wwwwwwwwwwwwww...",
    "...wbbbbbbbbwwwww...",
    "...wwwwwwwwwwwwww...",
    "...wkkkkkkkkkkkkw...",
    "...wkkkkkkkkwwwww...",
    "...wwwwwwwwwwwwww...",
    "...wkkkkkkkkkkkww...",
    "...wkkkkkkwwwwwww...",
    "...wwwwwwGGwwwwww...",
    "...wwwwwwwwwwwwww...",
];
/// Crumpled page stuck in the slot, with a red smudge.
const JAM: [&str; 5] = [
    "...wwkwwwwwkwwww....",
    "..w.wwwrrwwww.ww....",
    "...ww.wwwkww.w.w....",
    "....w.wwkw..w.......",
    "......w..w..........",
];

/// One animation frame.
const FRAME: u128 = 90;
/// Frames for one page to come out of the slot.
const PAGE: u128 = 11;

fn palette(c: char) -> Option<Color> {
    let i = match c {
        'w' => 231,
        'k' => 246,
        'b' => 69,
        'd' => 240,
        'g' => 252,
        's' => 234,
        'G' => 114,
        'r' => 203,
        'c' => 87,
        _ => return None,
    };
    Some(Color::Indexed(i))
}

/// Pixel rows to lines, two pixels per cell with half blocks, like install.sh's `render`.
pub fn pixels(rows: &[Vec<Option<Color>>], pad: usize) -> Vec<Line<'static>> {
    rows.chunks(2)
        .map(|pair| {
            let bottom = pair.get(1).cloned().unwrap_or_else(|| vec![None; pair[0].len()]);
            let mut spans = vec![Span::raw(" ".repeat(pad))];
            for (t, u) in pair[0].iter().zip(bottom) {
                spans.push(match (*t, u) {
                    (None, None) => Span::raw(" "),
                    (None, Some(u)) => Span::styled("▄", Style::new().fg(u)),
                    (Some(t), None) => Span::styled("▀", Style::new().fg(t)),
                    (Some(t), Some(u)) => Span::styled("▀", Style::new().fg(t).bg(u)),
                });
            }
            Line::from(spans)
        })
        .collect()
}

/// What the printer is reacting to.
#[derive(Clone, Copy)]
pub enum Act {
    Idle,
    /// A setting changed: a little hop.
    Hop(Instant),
    /// A job was sent: this many pages come out.
    Print(Instant, u32),
    /// Something went wrong: paper jam.
    Jam(Instant),
}

impl Act {
    /// The animation has played, so the next key press can bring the printer back to rest.
    pub fn done(&self) -> bool {
        match *self {
            Act::Idle => true,
            Act::Hop(at) => at.elapsed() > Duration::from_millis(300),
            Act::Print(at, n) => at.elapsed().as_millis() > FRAME * PAGE * n as u128,
            Act::Jam(at) => at.elapsed().as_millis() > FRAME * 10,
        }
    }
}

/// Background work the printer shows while it runs.
#[derive(Clone, Copy, PartialEq)]
pub enum Work {
    None,
    /// Scanning: a light sweeps across the body.
    Scan,
    /// Anything else: the lights blink fast.
    Busy,
}

/// The printer at this moment; `ms` is the time since the app started, for the idle blinking.
pub fn printer(act: Act, work: Work, ms: u128) -> Vec<Line<'static>> {
    let t = ms / FRAME;
    let mut lights = if (t / 8).is_multiple_of(2) { ('G', 'g') } else { ('g', 'G') };
    let (mut out, mut paper, mut pad) = (0, &PAPER[..], 2);
    match act {
        Act::Hop(at) if !act.done() => {
            pad = [1, 3, 2, 2][(at.elapsed().as_millis() / 60).min(3) as usize];
            lights = ('G', 'G');
        }
        Act::Print(at, n) => {
            let e = at.elapsed().as_millis() / FRAME;
            if e / PAGE < n as u128 {
                out = (e % PAGE) as usize;
                lights = if e.is_multiple_of(2) { ('G', 'g') } else { ('g', 'G') };
            } else {
                // the last page stays out until the next key
                (out, lights) = (10, ('G', 'G'));
            }
        }
        Act::Jam(at) => {
            let e = (at.elapsed().as_millis() / FRAME) as usize;
            (paper, out) = (&JAM[..], 5);
            pad = if e < 10 { [1, 3][e % 2] } else { 2 };
            lights = if e < 10 && e % 2 == 1 { ('g', 'g') } else { ('r', 'r') };
        }
        _ => {}
    }
    match work {
        Work::Busy => lights = if t.is_multiple_of(2) { ('G', 'g') } else { ('g', 'G') },
        Work::Scan => lights = ('G', 'G'),
        Work::None => {}
    }
    let mut rows: Vec<Vec<char>> = PRINTER
        .iter()
        .map(|r| r.chars().map(|c| match c {
            'L' => lights.0,
            'M' => lights.1,
            c => c,
        }).collect())
        .collect();
    if work == Work::Scan {
        let col = 2 + (t % 16) as usize;
        for row in &mut rows[4..7] {
            if row[col] == 'g' {
                row[col] = 'c';
            }
        }
    }
    rows.extend(paper[paper.len() - out..].iter().map(|r| r.chars().collect()));
    let art: Vec<Vec<Option<Color>>> = rows.iter().map(|r| r.iter().map(|c| palette(*c)).collect()).collect();
    pixels(&art, pad)
}

/// Ink tanks as (RGB color, percent or -1 when unknown), each filled up to its level, with the
/// percentages underneath. Four columns per tank.
pub fn tanks(inks: &[(u32, i32)]) -> Vec<Line<'static>> {
    const H: i32 = 10;
    let dark = Some(Color::Indexed(240));
    let rows: Vec<Vec<Option<Color>>> = (0..H)
        .map(|y| {
            inks.iter()
                .flat_map(|&(rgb, level)| {
                    let full = y > 0 && y < H - 1 && (H - 1 - y) * 100 / (H - 2) <= level;
                    let ink = Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
                    let fill = if y == 0 || y == H - 1 { dark } else if full { Some(ink) } else { None };
                    [dark, fill, dark, None]
                })
                .collect()
        })
        .collect();
    let mut lines = pixels(&rows, 0);
    let labels: String = inks.iter().map(|(_, l)| if *l < 0 { format!("{:^3} ", "?") } else { format!("{l:^3} ") }).collect();
    lines.push(Line::styled(labels, Style::new().fg(DIM)));
    lines
}

#[test]
fn printer_frames() {
    let at = Instant::now() - Duration::from_secs(60);
    // at rest: the printer alone, 10 pixel rows in 5 lines of 22 columns (with the padding)
    let rest = printer(Act::Idle, Work::None, 0);
    assert_eq!(rest.len(), 5);
    assert!(rest.iter().all(|l| l.width() == 22));
    // a finished print leaves the page out: 10 more pixel rows
    assert_eq!(printer(Act::Print(at, 2), Work::None, 0).len(), 10);
    assert!(Act::Print(at, 2).done() && !Act::Print(Instant::now(), 1).done());
    // a jam: the crumpled page, and red lights once the shaking stops
    let jam = printer(Act::Jam(at), Work::None, 0);
    assert_eq!(jam.len(), 8);
    assert!(jam[2].spans.iter().any(|s| s.style.fg == Some(RED) || s.style.bg == Some(RED)));
    // tanks: 4 columns each, a label line under them
    let t = tanks(&[(0x00FFFF, 50), (0, -1)]);
    assert_eq!(t.len(), 6);
    assert_eq!(t[0].width(), 8);
}
