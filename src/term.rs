use std::io::Write;
use colored::Colorize;
use byteorder::WriteBytesExt;

fn choose_glyph(custom: bool, g: char, v: f32) -> char {
    if custom { return g; }
    if g == ' ' || g == '/' || g == '-' || g == '_' || g == '\\' { return g; }
    if v < 0.2 {
        '.'
    } else if v < 0.4 {
        ','
    } else if v < 0.6 {
        '`'
    } else if v < 0.8 {
        '*'
    } else {
        '#'
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Foreground {
        custom_glyph: bool,
        glyph0: char,
        glyph1: Option<char>,
        color: (u8, u8, u8),
    },
    Background,
}

impl Cell {
    pub fn serialize<W>(&self, out: &mut W) -> Option<()>
    where W: Write
    {
        match self {
            Self::Foreground { custom_glyph, glyph0, glyph1, color } => {
                out.write_u8(1).ok()?;
                out.write_u8(if *custom_glyph { 1 } else { 0 }).ok()?;
                out.write_u8(color.0).ok()?;
                out.write_u8(color.1).ok()?;
                out.write_u8(color.2).ok()?;
                out.write_u32::<byteorder::BigEndian>(*glyph0 as _).ok()?;
                if let Some(g1) = *glyph1 {
                    out.write_u8(1).ok()?;
                    out.write_u32::<byteorder::BigEndian>(g1 as _).ok()?;
                } else {
                    out.write_u8(0).ok()?;
                }
            },
            Self::Background => {
                out.write_u8(0).ok()?;
            },
        }
        Some(())
    }
}

pub struct Diff {
    pub diff: Vec<(u8, u8, Cell)>,
}

impl Diff {
    pub fn new(old: &Term, new: &Term) -> Self {
        let mut diff = Vec::new();
        let h = old.h.min(new.h);
        let w = old.w.min(new.w);
        for y in 0..h {
            for x in 0..w {
                if old.cells[y][x] != new.cells[y][x] {
                    diff.push((x as _, y as _, new.cells[y][x]));
                }
            }
        }
        Self {
            diff,
        }
    }

    pub fn serialize<W>(&self, out: &mut W)
    where W: Write
    {
        out.write_u8(1).unwrap();
        out.write_u32::<byteorder::BigEndian>(self.diff.len() as _).unwrap();
        for (x, y, c) in self.diff.iter() {
            out.write_u8(*x).unwrap();
            out.write_u8(*y).unwrap();
            c.serialize(out).unwrap();
        }
    }

    pub fn serialize_and_compress(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        {
            let mut writer = flate2::write::GzEncoder::new(&mut ret, flate2::Compression::default());
            self.serialize(&mut writer);
        }
        ret
    }
}

pub struct Term {
    pub count: usize,
    pub w: usize,
    pub h: usize,
    pub cells: Vec<Vec<Cell>>,
}

impl Term {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            count: 0,
            w,
            h,
            cells: vec![vec![Cell::Background; w]; h],
        }
    }

    /// 终端尺寸变化时重建网格（不换对象，方便上层拿着 &mut 引用）
    pub fn resize(&mut self, w: usize, h: usize) {
        if self.w == w && self.h == h { return }
        self.w = w;
        self.h = h;
        self.cells = vec![vec![Cell::Background; w]; h];
    }

    pub fn write(&mut self, x: usize, y: usize, custom_glyph: bool, glyph0: char, glyph1: Option<char>, color: (u8, u8, u8)) {
        if y < self.h && x < self.w {
            self.cells[y][x] = Cell::Foreground { custom_glyph, glyph0, glyph1, color, };
        }
    }

    pub fn serialize<W>(&self, out: &mut W)
    where W: Write
    {
        out.write_u8(0).unwrap();
        for row in self.cells.iter() {
            for cell in row.iter() {
                cell.serialize(out).unwrap();
            }
        }
    }

    pub fn serialize_and_compress(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        {
            let mut writer = flate2::write::GzEncoder::new(&mut ret, flate2::Compression::default());
            self.serialize(&mut writer);
        }
        ret
    }

    /// 把整帧编码成 ANSI 字节流（不写出去）。
    /// 主循环用这个 + 独立写线程，避免 pty 写满时把渲染循环阻塞住。
    pub fn encode_stream(&self, bg: (u8, u8, u8)) -> Vec<u8> {
        self.encode_stream_sized(crate::display::Size::new(self.w, self.h), bg)
    }

    /// Downsample only the terminal output, centered and letterboxed; the render/stream grid stays intact.
    /// Absolute row positions and no trailing newline prevent wrapping/scrolling on small terminals.
    pub fn encode_stream_sized(&self, bounds: crate::display::Size, bg: (u8, u8, u8)) -> Vec<u8> {
        let content = crate::display::contain(crate::display::Size::new(self.w, self.h), bounds);
        let left = (bounds.w - content.w) / 2;
        let top = (bounds.h - content.h) / 2;
        let mut output = Vec::new();
        write!(output, "\x1b[?2026h\x1b[?25l\x1b[48;2;{};{};{}m", bg.0, bg.1, bg.2).unwrap();
        let mut current_color = None;
        for y in 0..bounds.h {
            write!(output, "\x1b[{};1H", y + 1).unwrap();
            for x in 0..bounds.w {
                let (glyph, color) = if x >= left && x < left + content.w
                    && y >= top && y < top + content.h {
                    let sx = ((x - left) * self.w / content.w).min(self.w - 1);
                    let sy = ((y - top) * self.h / content.h).min(self.h - 1);
                    match self.cells[sy][sx] {
                        Cell::Foreground { glyph0, color, .. } => (glyph0, color),
                        Cell::Background => (' ', bg),
                    }
                } else { (' ', bg) };
                if current_color != Some(color) {
                    write!(output, "\x1b[38;2;{};{};{}m", color.0, color.1, color.2).unwrap();
                    current_color = Some(color);
                }
                write!(output, "{}", glyph).unwrap();
            }
            // Erase stale columns from the previous wider frame, keeping the configured background.
            write!(output, "\x1b[K").unwrap();
        }
        write!(output, "\x1b[J\x1b[?2026l").unwrap();
        output
    }

    /// 导出当前帧的字符快照，给本地推流服务用
    pub fn snapshot(&self) -> Vec<Vec<(char, (u8, u8, u8))>> {
        self.cells.iter().map(|row| {
            row.iter().map(|cell| match cell {
                Cell::Foreground { glyph0, color, .. } => (*glyph0, *color),
                Cell::Background => (' ', (0, 0, 0)),
            }).collect()
        }).collect()
    }

    pub fn render_stream<W>(&self, out: &mut W, bg: (u8, u8, u8))
    where W: Write {
        let buf = self.encode_stream(bg);
        out.write(&buf).unwrap();
    }

    pub fn render_stream_nocolor<W>(&self, out: &mut W)
    where W: Write {
        let mut output: Vec<u8> = Vec::new();
        write!(output, "\x1b[2J\x1b[1;1H").unwrap();
        for row in self.cells.iter() {
            for cell in row.iter() {
                match cell {
                    Cell::Foreground { custom_glyph, glyph0, glyph1, color, } => {
                        let r: f32 = (color.0 as f32) / 255.0;
                        let g: f32 = (color.1 as f32) / 255.0;
                        let b: f32 = (color.2 as f32) / 255.0;
                        let mono = (0.2125 * r) + (0.7154 * g) + (0.0721 * b);
                        let s = if let Some(g) = glyph1 {
                            format!(
                                "{}{}",
                                choose_glyph(*custom_glyph, *glyph0, mono),
                                choose_glyph(*custom_glyph, *g, mono)
                            )
                        } else {
                            format!(
                                "{}",
                                choose_glyph(*custom_glyph, *glyph0, mono)
                            )
                        };
                        write!(
                            output, "{}",
                            s
                        ).unwrap();
                    },
                    Cell::Background => {},
                }
            }
            write!(output, "\r\n").unwrap();
        }
        out.write(&output).unwrap();
    }

    pub fn render_stream_nocolor_small<W>(&self, out: &mut W)
    where W: Write {
        let mut output: Vec<u8> = Vec::new();
        write!(output, "\x1b[2J\x1b[1;1H").unwrap();
        for row in self.cells.iter().step_by(2) {
            for cell in row.iter().step_by(2) {
                match cell {
                    Cell::Foreground { custom_glyph, glyph0, glyph1, color, } => {
                        let r: f32 = (color.0 as f32) / 255.0;
                        let g: f32 = (color.1 as f32) / 255.0;
                        let b: f32 = (color.2 as f32) / 255.0;
                        let mono = (0.2125 * r) + (0.7154 * g) + (0.0721 * b);
                        let s = if let Some(g) = glyph1 {
                            format!(
                                "{}{}",
                                choose_glyph(*custom_glyph, *glyph0, mono),
                                choose_glyph(*custom_glyph, *g, mono)
                            )
                        } else {
                            format!(
                                "{}",
                                choose_glyph(*custom_glyph, *glyph0, mono)
                            )
                        };
                        write!(
                            output, "{}",
                            s
                        ).unwrap();
                    },
                    Cell::Background => {},
                }
            }
            write!(output, "\r\n").unwrap();
        }
        out.write(&output).unwrap();
    }
}


#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::display::Size;

    #[test]
    fn small_terminal_is_safe_and_snapshot_keeps_source_resolution() {
        let mut term = Term::new(128, 64);
        term.write(64, 32, false, '@', None, (254, 200, 101));
        let bytes = term.encode_stream_sized(Size::new(46, 20), (0, 0, 0));
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains('\n'));
        assert!(!text.contains("\x1b[21;1H"));
        assert!(text.contains("\x1b[20;1H"));
        let snapshot = term.snapshot();
        assert_eq!(snapshot.len(), 64);
        assert_eq!(snapshot[0].len(), 128);
        assert_eq!(snapshot[32][64], ('@', (254, 200, 101)));
    }

    #[test]
    fn full_size_output_keeps_rgb_values() {
        let mut term = Term::new(2, 1);
        term.write(1, 0, false, 'A', None, (255, 128, 254));
        let text = String::from_utf8(term.encode_stream((21, 5, 15))).unwrap();
        assert!(text.contains("\x1b[48;2;21;5;15m"));
        assert!(text.contains("\x1b[38;2;255;128;254mA"));
        assert!(!text.contains('\n'));
    }
}
