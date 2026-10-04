//! The invisible text of a searchable PDF in any language: a font whose glyphs draw nothing
//! (as tesseract's GlyphLessFont), one glyph per letter used, and a ToUnicode map back to the
//! letters, so viewers can search and copy Chinese, Hindi, Arabic or Russian as well as Latin.

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo, UnicodeCmap};
use pdf_writer::{Name, Pdf, Rect, Ref, Str};

/// Each glyph is half an em wide; the text is stretched to each word's box anyway.
pub const GLYPH_WIDTH: f32 = 500.0;

/// The letters of a text layer, each with its glyph (its character id, CID): 1, 2, 3...
#[derive(Default)]
pub struct Letters(Vec<char>);

impl Letters {
    /// The text as glyph ids, two bytes each, as the font's Identity-H encoding shows them;
    /// new letters get the next glyph.
    pub fn encode(&mut self, text: &str) -> Vec<u8> {
        text.chars()
            .flat_map(|c| {
                let i = self.0.iter().position(|&l| l == c).unwrap_or_else(|| {
                    self.0.push(c);
                    self.0.len() - 1
                });
                // glyph 0 is .notdef, and there are at most 65535
                u16::try_from(i + 1).unwrap_or(0).to_be_bytes()
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Writes the Type0 font `font` and the objects it needs, with ids from `next` on.
    pub fn write(&self, pdf: &mut Pdf, font: Ref, next: i32) {
        let (cid, descriptor, file, to_unicode) = (Ref::new(next), Ref::new(next + 1), Ref::new(next + 2), Ref::new(next + 3));
        let name = Name(b"GlyphLessFont");
        let info = SystemInfo { registry: Str(b"Adobe"), ordering: Str(b"Identity"), supplement: 0 };
        pdf.type0_font(font).base_font(name).encoding_predefined(Name(b"Identity-H")).descendant_font(cid).to_unicode(to_unicode);
        pdf.cid_font(cid)
            .subtype(CidFontType::Type2)
            .base_font(name)
            .system_info(info)
            .font_descriptor(descriptor)
            .default_width(GLYPH_WIDTH)
            .cid_to_gid_map_predefined(Name(b"Identity"));
        pdf.font_descriptor(descriptor)
            .name(name)
            .flags(FontFlags::SYMBOLIC | FontFlags::FIXED_PITCH)
            .bbox(Rect::new(0.0, 0.0, GLYPH_WIDTH, 1000.0))
            .italic_angle(0.0)
            .ascent(1000.0)
            .descent(0.0)
            .cap_height(1000.0)
            .stem_v(80.0)
            .font_file2(file);
        let ttf = glyphless_font(self.0.len().min(65534) as u16 + 1);
        pdf.stream(file, &ttf).pair(Name(b"Length1"), ttf.len() as i32);
        let mut cmap = UnicodeCmap::new(Name(b"PrinterTUI-UTF16"), info);
        for (i, &c) in self.0.iter().take(65534).enumerate() {
            cmap.pair(i as u16 + 1, c);
        }
        pdf.cmap(to_unicode, &cmap.finish());
    }
}

/// A TrueType font of `glyphs` glyphs, every one empty and half an em wide.
pub fn glyphless_font(glyphs: u16) -> Vec<u8> {
    let n = glyphs.max(1);
    let be16 = |v: &[u16]| -> Vec<u8> { v.iter().flat_map(|x| x.to_be_bytes()).collect() };
    let width = GLYPH_WIDTH as u16;

    let mut head = Vec::new();
    head.extend(0x0001_0000u32.to_be_bytes()); // version
    head.extend(0x0001_0000u32.to_be_bytes()); // font revision
    head.extend(0u32.to_be_bytes()); // checksum adjustment, filled in below
    head.extend(0x5F0F_3CF5u32.to_be_bytes()); // magic
    head.extend(be16(&[0b1011, 1000])); // flags, units per em
    // created and modified: 2026-01-01, in seconds since 1904
    head.extend([3_850_070_400u64.to_be_bytes(), 3_850_070_400u64.to_be_bytes()].concat());
    head.extend(be16(&[0, 0, width, 1000, 0, 3, 2, 0, 0])); // box, style, smallest size, direction, short loca, glyph format

    let mut hhea = Vec::new();
    hhea.extend(0x0001_0000u32.to_be_bytes());
    // ascender, descender, line gap, widest, min left and right bearings, extent, caret (rise, run, offset), 4 reserved, format, metrics
    hhea.extend(be16(&[1000, 0, 0, width, 0, 0, width, 1, 0, 0, 0, 0, 0, 0, 0, 1]));

    let mut maxp = Vec::new();
    maxp.extend(0x0001_0000u32.to_be_bytes());
    // glyphs, then no points, contours, composites, instructions; 2 zones
    maxp.extend(be16(&[n, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]));

    // one width for all glyphs, then their left side bearings
    let mut hmtx = be16(&[width, 0]);
    hmtx.extend(vec![0; 2 * (n as usize - 1)]);
    // every glyph empty: they all start and end at 0, in an empty glyph table
    let loca = vec![0; 2 * (n as usize + 1)];
    let glyf = Vec::new();

    // a Windows Unicode map that maps nothing: the PDF picks glyphs by id
    let mut cmap = be16(&[0, 1, 3, 1, 0, 12]);
    cmap.extend(be16(&[4, 24, 0, 2, 2, 0, 0, 0xFFFF, 0, 0xFFFF, 1, 0]));

    let mut name = Vec::new();
    let names: [(u16, &str); 4] = [(1, "GlyphLessFont"), (2, "Regular"), (4, "GlyphLessFont"), (6, "GlyphLessFont")];
    let strings: Vec<Vec<u8>> = names.iter().map(|(_, s)| be16(&s.encode_utf16().collect::<Vec<_>>())).collect();
    name.extend(be16(&[0, names.len() as u16, 6 + 12 * names.len() as u16]));
    let mut offset = 0;
    for ((id, _), s) in names.iter().zip(&strings) {
        name.extend(be16(&[3, 1, 0x409, *id, s.len() as u16, offset]));
        offset += s.len() as u16;
    }
    name.extend(strings.concat());

    let mut post = Vec::new();
    post.extend(0x0003_0000u32.to_be_bytes()); // version 3: no glyph names
    post.extend(0u32.to_be_bytes()); // italic angle
    post.extend(be16(&[(-100i16) as u16, 50]));
    post.extend(1u32.to_be_bytes()); // fixed pitch
    post.extend([0; 16]);

    let mut os2 = be16(&[4, width, 400, 5, 0, 650, 600, 0, 75, 650, 600, 0, 350, 50, 300, 0]);
    os2.extend([0; 10 + 16]); // panose, unicode ranges
    os2.extend(*b"NONE");
    os2.extend(be16(&[0x40, 0xFFFF, 0xFFFF, 1000, 0, 0, 1000, 0]));
    os2.extend(1u32.to_be_bytes());
    os2.extend(0u32.to_be_bytes());
    os2.extend(be16(&[500, 1000, 0, 0x20, 0]));

    let mut tables: [(&[u8; 4], Vec<u8>); 10] = [
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    let checksum = |data: &[u8]| -> u32 {
        data.chunks(4)
            .map(|c| {
                let mut w = [0; 4];
                w[..c.len()].copy_from_slice(c);
                u32::from_be_bytes(w)
            })
            .fold(0u32, u32::wrapping_add)
    };
    let count = tables.len() as u16;
    let level = 15 - count.leading_zeros() as u16;
    let mut font = Vec::new();
    font.extend(0x0001_0000u32.to_be_bytes());
    font.extend(be16(&[count, 16 << level, level, count * 16 - (16 << level)]));
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in tables.iter_mut() {
        font.extend(*tag);
        font.extend(checksum(data).to_be_bytes());
        font.extend((offset as u32).to_be_bytes());
        font.extend((data.len() as u32).to_be_bytes());
        data.resize(data.len().next_multiple_of(4), 0);
        offset += data.len();
        body.extend_from_slice(data);
    }
    font.extend(body);
    // the whole font then sums to 0xB1B0AFBA, as the head table says
    let head_at = 12 + 16 * tables.len() + tables[..3].iter().map(|(_, d)| d.len()).sum::<usize>();
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
    font[head_at + 8..head_at + 12].copy_from_slice(&adjust.to_be_bytes());
    font
}

#[test]
fn letters_get_glyphs_in_order() {
    let mut letters = Letters::default();
    assert_eq!(letters.encode("añ中"), [0, 1, 0, 2, 0, 3]);
    assert_eq!(letters.encode("中a"), [0, 3, 0, 1]);
    let font = glyphless_font(4);
    assert_eq!(&font[..4], &[0, 1, 0, 0]);
    // every table four-byte aligned and inside the font
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    for t in 0..count {
        let rec = &font[12 + 16 * t..28 + 16 * t];
        let (offset, len) = (u32::from_be_bytes(rec[8..12].try_into().unwrap()) as usize, u32::from_be_bytes(rec[12..16].try_into().unwrap()) as usize);
        assert!(offset % 4 == 0 && offset + len <= font.len(), "{}", String::from_utf8_lossy(&rec[..4]));
    }
}
