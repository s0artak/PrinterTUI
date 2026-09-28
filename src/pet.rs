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

// . empty  w paper  o paper edge  k ink  b blue  d dark body  g light body  s slot  G green
// r red  c cyan  L M the two lights. The paper has a gray edge, or it vanishes on a light terminal.
const PRINTER: [&str; 10] = [
    "......oooooooo......",
    "......owwwwwwo......",
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
    "...owwwwwwwwwwwwo...",
    "...obbbbbbbbwwwwo...",
    "...owwwwwwwwwwwwo...",
    "...okkkkkkkkkkkko...",
    "...okkkkkkkkwwwwo...",
    "...owwwwwwwwwwwwo...",
    "...okkkkkkkkkkkwo...",
    "...okkkkkkwwwwwwo...",
    "...owwwwwGGwwwwwo...",
    "...oooooooooooooo...",
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
        'o' => 248,
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

/// How the real printer is doing, shown while nothing else happens.
#[derive(Clone, Copy, PartialEq)]
pub enum Mood {
    Fine,
    /// Out of paper: the tray is empty and a light blinks red.
    NoPaper,
    /// Offline or paused: the lights are off.
    Off,
    /// Anything else that needs a person: red lights.
    Trouble,
}

/// The printer at this moment; `ms` is the time since the app started, for the idle blinking.
pub fn printer(act: Act, work: Work, mood: Mood, ms: u128) -> Vec<Line<'static>> {
    let t = ms / FRAME;
    let mut lights = match mood {
        Mood::Fine if (t / 8).is_multiple_of(2) => ('G', 'g'),
        Mood::Fine => ('g', 'G'),
        Mood::NoPaper if (t / 8).is_multiple_of(2) => ('r', 'g'),
        Mood::NoPaper => ('g', 'g'),
        Mood::Off => ('d', 'd'),
        Mood::Trouble => ('r', 'r'),
    };
    let calm = matches!(act, Act::Idle | Act::Hop(_)) && work == Work::None;
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
    if calm && mood == Mood::NoPaper {
        // the paper in the tray is gone
        for row in &mut rows[0..2] {
            row.iter_mut().for_each(|c| *c = '.');
        }
    }
    rows.extend(paper[paper.len() - out..].iter().map(|r| r.chars().collect()));
    let art: Vec<Vec<Option<Color>>> = rows.iter().map(|r| r.iter().map(|c| palette(*c)).collect()).collect();
    pixels(&art, pad)
}

/// The printed sheet in the flip animation, its top edge green so you can follow it.
const FRONT: [&str; 12] = [
    "oGGGGGGGGGGo",
    "owbbbbbbwwwo",
    "owwwwwwwwwwo",
    "owkkkkkkkkwo",
    "owkkkkkwwwwo",
    "owwwwwwwwwwo",
    "owkkkkkkkkwo",
    "owkkkkkkwwwo",
    "owwwwwwwwwwo",
    "owkkkkwwwwwo",
    "owwwwwwwwwwo",
    "oooooooooooo",
];
/// Its blank back once flipped top over bottom: the top edge is now at the bottom.
const BACK: [&str; 12] = [
    "oooooooooooo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "owwwwwwwwwwo",
    "oGGGGGGGGGGo",
];

/// Steps of the flip animation, as (sheet top, sheet height, printed side showing, caption):
/// the sheet turns top over bottom, then slides into the tray on top of the printer.
const FLIP: [(usize, usize, bool, usize); 15] = [
    (0, 12, true, 0),
    (0, 12, true, 0),
    (2, 8, true, 1),
    (4, 4, true, 1),
    (5, 2, true, 1),
    (4, 4, false, 1),
    (2, 8, false, 1),
    (0, 12, false, 2),
    (0, 12, false, 2),
    (3, 12, false, 3),
    (6, 12, false, 3),
    (9, 12, false, 3),
    (12, 12, false, 3),
    (14, 12, false, 4),
    (14, 12, false, 4),
];

/// One frame of the manual duplex animation, and which of the five captions goes with it:
/// printed side facing you, flip it, blank side facing you, into the tray, press Enter.
pub fn flip(step: usize) -> (Vec<Line<'static>>, usize) {
    let (top, height, printed, caption) = FLIP[step % FLIP.len()];
    let sheet = if printed { &FRONT } else { &BACK };
    // 12 rows above the printer for the sheet, then the printer, which hides the sheet going in
    let mut canvas = vec![vec!['.'; 20]; 22];
    for i in 0..height {
        // a squashed sheet shows its rows evenly sampled
        let row = sheet[(2 * i + 1) * 12 / (2 * height)];
        let y = top + (12 - height) / 2 + i;
        if let Some(line) = canvas.get_mut(y) {
            line[4..16].iter_mut().zip(row.chars()).for_each(|(c, p)| *c = p);
        }
    }
    let lights = if caption == 4 { 'G' } else { 'g' };
    for (y, row) in PRINTER.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c != '.' {
                canvas[12 + y][x] = match c {
                    'L' | 'M' => lights,
                    c => c,
                };
            }
        }
    }
    let art: Vec<Vec<Option<Color>>> = canvas.iter().map(|r| r.iter().map(|c| palette(*c)).collect()).collect();
    (pixels(&art, 0), caption)
}

/// Ink tanks as (RGB color, percent or -1 when unknown, running low), each filled up to its
/// level, with the percentages underneath (red when low). Four columns per tank.
pub fn tanks(inks: &[(u32, i32, bool)]) -> Vec<Line<'static>> {
    const H: i32 = 10;
    let dark = Some(Color::Indexed(240));
    let rows: Vec<Vec<Option<Color>>> = (0..H)
        .map(|y| {
            inks.iter()
                .flat_map(|&(rgb, level, _)| {
                    let full = y > 0 && y < H - 1 && (H - 1 - y) * 100 / (H - 2) <= level;
                    let ink = Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
                    let fill = if y == 0 || y == H - 1 { dark } else if full { Some(ink) } else { None };
                    [dark, fill, dark, None]
                })
                .collect()
        })
        .collect();
    let mut lines = pixels(&rows, 0);
    let labels: Vec<Span> = inks
        .iter()
        .map(|&(_, l, low)| {
            let text = if l < 0 { format!("{:^3} ", "?") } else { format!("{l:^3} ") };
            Span::styled(text, Style::new().fg(if low { RED } else { DIM }))
        })
        .collect();
    lines.push(Line::from(labels));
    lines
}

#[test]
fn printer_frames() {
    let at = Instant::now() - Duration::from_secs(60);
    // at rest: the printer alone, 10 pixel rows in 5 lines of 22 columns (with the padding)
    let rest = printer(Act::Idle, Work::None, Mood::Fine, 0);
    assert_eq!(rest.len(), 5);
    assert!(rest.iter().all(|l| l.width() == 22));
    // a finished print leaves the page out: 10 more pixel rows
    assert_eq!(printer(Act::Print(at, 2), Work::None, Mood::Fine, 0).len(), 10);
    // out of paper: the tray on top is empty
    let empty = printer(Act::Idle, Work::None, Mood::NoPaper, 0);
    assert!(empty[0].spans.iter().all(|s| s.content.trim().is_empty()));
    assert!(Act::Print(at, 2).done() && !Act::Print(Instant::now(), 1).done());
    // a jam: the crumpled page, and red lights once the shaking stops
    let jam = printer(Act::Jam(at), Work::None, Mood::Fine, 0);
    assert_eq!(jam.len(), 8);
    assert!(jam[2].spans.iter().any(|s| s.style.fg == Some(RED) || s.style.bg == Some(RED)));
    // the flip: 11 lines, starting with the printed side and ending in "press Enter"
    let (first, caption) = flip(0);
    assert_eq!((first.len(), caption), (11, 0));
    assert_eq!(flip(14).1, 4);
    // the tanks: 4 columns each, a label line under them
    let t = tanks(&[(0x00FFFF, 50, false), (0, -1, false)]);
    assert_eq!(t.len(), 6);
    assert_eq!(t[0].width(), 8);
}
