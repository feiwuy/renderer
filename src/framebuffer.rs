use std::io::Write;
use termion::raw::IntoRawMode;
use colored::Colorize;

use crate::{context, utils, gl, avatar, term};

pub struct Framebuffer {
    pub tex: gl::types::GLuint,
    pub fbo: gl::types::GLuint,
    pub dims: utils::Dimensions,
    pub offsets: (i32, i32),
    pub pixels_len: usize,
    pub pixels: Vec<u8>,
}

impl Framebuffer {
    pub fn new(_ctx: &context::Context, depth: bool, dims: (i32, i32), offsets: (i32, i32)) -> Self {
        let (w, h) = dims;
        let mut tex: gl::types::GLuint = 0;
        let mut fbo: gl::types::GLuint = 0;
        let mut depth_buffer: gl::types::GLuint = 0;
        unsafe {
            // generate and bind FBO
            gl::GenFramebuffers(1, &mut fbo);
            gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);

            if depth {
                // generate and attach depth buffer
                gl::GenRenderbuffers(1, &mut depth_buffer);
                gl::BindRenderbuffer(gl::RENDERBUFFER, depth_buffer);
                gl::RenderbufferStorage(gl::RENDERBUFFER, gl::DEPTH_COMPONENT32F, w, h);
                gl::FramebufferRenderbuffer(gl::FRAMEBUFFER, gl::DEPTH_ATTACHMENT, gl::RENDERBUFFER, depth_buffer);
            }

            // generate and attach texture
            gl::GenTextures(1, &mut tex);
            gl::BindTexture(gl::TEXTURE_2D, tex);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                w,
                h,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                0 as _,
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::FramebufferTexture2D(gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::TEXTURE_2D, tex, 0);
            gl::DrawBuffer(gl::COLOR_ATTACHMENT0);

            let status = gl::CheckFramebufferStatus(gl::FRAMEBUFFER);
            if status != gl::FRAMEBUFFER_COMPLETE {
                panic!("error initializing framebuffer: {}", status);
            }
        }
        let pixels_len = (w * h * 4) as usize;
        let pixels = vec![0; pixels_len];
        Self {
            tex,
            fbo,
            dims: utils::Dimensions { w: w as _, h: h as _ },
            offsets,
            pixels,
            pixels_len,
        }
    }

    /// 网格尺寸变化时重建 FBO（旧的 GL 对象直接丢弃，resize 不常发生）
    pub fn resize(&mut self, ctx: &context::Context, w: i32, h: i32) {
        if self.dims.w == w as f32 && self.dims.h == h as f32 { return }
        let nf = Framebuffer::new(ctx, true, (w, h), self.offsets);
        self.fbo = nf.fbo;
        self.tex = nf.tex;
        self.dims = nf.dims;
        self.pixels_len = nf.pixels_len;
        self.pixels = nf.pixels;
    }

    pub fn bind(&self, _ctx: &context::Context) {
        let (offsetx, offsety) = self.offsets;
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.fbo);
            gl::Viewport(offsetx as _, offsety as _, self.dims.w as _, self.dims.h as _);
            // gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);

        }
    }

    /// 注意：这里的 x 是**行**索引（0..h，回读来自下往上所以要翻转），y 是**列**索引（0..w）。
    /// 原版索引公式乘的是 h，只在正方形缓冲下成立；这里改成乘 w，支持 128x64 这种宽屏网格。
    pub fn get_pixel(&self, bg: (i32, i32, i32), x: i32, y: i32) -> Option<(u8, u8, u8)> {
        self.get_pixel_rgba(bg, x, y).map(|(r, g, b, _)| (r, g, b))
    }

    /// 和 get_pixel 一样，但额外返回 alpha（启用 COLONQ_PARTS 时，alpha 里存的是部位编号）
    pub fn get_pixel_rgba(&self, bg: (i32, i32, i32), x: i32, y: i32) -> Option<(u8, u8, u8, u8)> {
        let w = self.dims.w as i32;
        let h = self.dims.h as i32;
        if x < 0 || x >= h || y < 0 || y >= w {
            None
        } else {
            let invx = h - (x + 1);
            let base_idx = ((invx * w + y) * 4) as usize;
            if base_idx > (w * h * 4) as usize { return None; }
            let r = *self.pixels.get(base_idx)?;
            let g = *self.pixels.get(base_idx + 1)?;
            let b = *self.pixels.get(base_idx + 2)?;
            let a = *self.pixels.get(base_idx + 3)?;
            if r == (bg.0 as u8) && g == (bg.1 as u8) && b == (bg.2 as u8) {
                None
            } else {
                Some((r, g, b, a))
            }
        }
    }

    pub fn get_surrounding(&self, bg: (i32, i32, i32), x: i32, y: i32) -> u8 {
        let mut ret = 0;
        if let Some(_) = self.get_pixel(bg, x - 1, y - 1) { ret = ret | 0b10000000 }
        if let Some(_) = self.get_pixel(bg, x - 1, y + 0) { ret = ret | 0b01000000 }
        if let Some(_) = self.get_pixel(bg, x - 1, y + 1) { ret = ret | 0b00100000 }
        if let Some(_) = self.get_pixel(bg, x + 0, y - 1) { ret = ret | 0b00010000 }
        if let Some(_) = self.get_pixel(bg, x + 0, y + 1) { ret = ret | 0b00001000 }
        if let Some(_) = self.get_pixel(bg, x + 1, y - 1) { ret = ret | 0b00000100 }
        if let Some(_) = self.get_pixel(bg, x + 1, y + 0) { ret = ret | 0b00000010 }
        if let Some(_) = self.get_pixel(bg, x + 1, y + 1) { ret = ret | 0b00000001 }
        ret
    }

    pub fn populate_pixels(&mut self) {
        unsafe {
            gl::GetTextureImage(self.tex, 0, gl::RGBA, gl::UNSIGNED_BYTE, self.pixels_len as _, self.pixels.as_mut_ptr() as _);
        }
    }

    pub fn render_term(&self, ctx: &context::Context, framecount: i32, players: &avatar::PalettePlayers, palette: &avatar::Palette, t: &mut term::Term, c: usize) {
        t.count = c;
        let mut idx = 0;
        let mut emoji_cache: std::collections::HashMap<char, bool> = std::collections::HashMap::new();
        for x in 0..(self.dims.h as _) {   // x = 行
            for y in 0..(self.dims.w as _) {   // y = 列
                match self.get_pixel_rgba(ctx.bgcolor, x, y) {
                    Some((cr, cg, cb, ca)) => {
                        let col = (cr, cg, cb);
                        let (mut custom, mut wordref, pal) = palette.lookup(col);
                        // 按部位换字符表（部位编号存在 alpha 里，0=没参与部位 pass）
                        let part_word = palette.part_words.get(&ca);
                        if let Some(pw) = part_word {
                            custom = true;
                            wordref = pw;
                        }
                        let (r, g, b) = match pal {
                            avatar::PaletteEntry::Color(c) => c,
                            avatar::PaletteEntry::Pattern { width, height, pixels } =>
                                if let Some(pcol) = pixels.get(((x % height) * width + (y % width)) as usize) {
                                    *pcol
                                } else {
                                    col
                                },
                            avatar::PaletteEntry::Animation { delay, width, height, frames } => {
                                let pixels: &Vec<(u8, u8, u8)> = frames.get(((framecount / delay.max(1)) % frames.len() as i32) as usize).unwrap();
                                if let Some(pcol) = pixels.get(((x % height) * width + (y % width)) as usize) {
                                    *pcol
                                } else {
                                    col
                                }
                            },
                            avatar::PaletteEntry::Video => {
                                context::PaletteType::from_color(col)
                                    .as_ref().and_then(|pty| players.players.get(pty))
                                    .and_then(|player| Some(player.get_pixel(x, y)))
                                    .unwrap_or(col)
                            },
                        };

                        let wordlen = wordref.chars().count();
                        let mut c1 = wordref.chars().nth(idx % wordlen).unwrap();
                        // emoji 查表带一次 String 分配，逐格做很贵；这里按字符缓存
                        let mut is_emoji = *emoji_cache
                            .entry(c1)
                            .or_insert_with(|| emojis::get(&format!("{}", c1)).is_some());
                        idx = idx + 1;
                        // 一格只有一个字符了，c2 只给边缘图案当占位用
                        let mut c2 = ' ';

                        let s = self.get_surrounding(ctx.bgcolor, x, y);
                        if s == 0b01101011 || s == 0b01101111 {
                            c1 = '|'; is_emoji = false;
                        } else if s == 0b11010110 || s == 0b11010110 {
                            if is_emoji { c1 = '.'; }
                            c2 = '|'; is_emoji = false;
                        } else if s == 0b00101011 || s == 0b00101111 || s == 0b00101110 || s == 0b00101100 || s == 0b00001011 {
                            c1 = ' '; c2 = '/'; is_emoji = false;
                        } else if s == 0b10010110 || s == 0b10010111 || s == 0b10010011 || s == 0b10010001 || s == 0b00010110 {
                            c1 = '\\'; c2 = ' '; is_emoji = false;
                        } else if s == 0b00111111 {
                            c1 = '-'; c2 = '/'; is_emoji = false;
                        } else if s == 0b10011111 {
                            c1 = '\\'; c2 = '-'; is_emoji = false;
                        } else if s == 0b00011111 {
                            c1 = '-'; c2 = '-'; is_emoji = false;
                        } else if s == 0b00001111 {
                            c1 = ' '; c2 = '_'; is_emoji = false;
                        } else if s == 0b00010111 {
                            c1 = '_'; c2 = ' '; is_emoji = false;
                        } else if s == 0b00000111 {
                            c1 = '_'; c2 = '_'; is_emoji = false;
                        }

                        // 现在是一个像素对一个字符（128 列），
                        // 所以边缘图案那对字符里挑更有信息量的那个
                        let ch = if c1 == ' ' && c2 != ' ' { c2 } else { c1 };
                        t.write(y as _, x as _, custom, ch, None, (r, g, b));
                    },
                    None => {
                        idx = idx + 1;
                        t.write(y as _, x as _, false, ' ', None, (0, 0, 0));
                    },
                }
            }
        }
    }

    pub fn render_ascii(&mut self, ctx: &context::Context, framecount: i32, players: &avatar::PalettePlayers, palette: &avatar::Palette, forsen: bool) {
        let mut stdout = std::io::stdout().into_raw_mode().unwrap();

        let bg = if forsen {
            colored::customcolors::CustomColor::new(
                0x2c,
                0x52,
                0x39,
            )
        } else {
            colored::customcolors::CustomColor::new(
                ctx.bgcolor.0 as u8,
                ctx.bgcolor.1 as u8,
                ctx.bgcolor.2 as u8,
            )
        };
        let bgcustom = if forsen {
            colored::customcolors::CustomColor::new(
                bg.r / 2,
                bg.g / 2,
                bg.b / 2,
            )
        } else {
            colored::customcolors::CustomColor::new(
                bg.r + 1,
                bg.g + 1,
                bg.b + 1,
            )
        };

        if (framecount % 6) == 0 {
            self.populate_pixels();
            print!("\x1b[2J\x1b[1;1H");
            let mut idx = 0;
            let mut output: Vec<u8> = Vec::new();
            for x in 0..(self.dims.w as _) {
                for y in 0..(self.dims.h as _) {
                    match self.get_pixel(ctx.bgcolor, x, y) {
                        Some(col) => {
                            let (_, wordref, pal) = palette.lookup(col);
                            let (r, g, b) = match pal {
                                avatar::PaletteEntry::Color(c) => c,
                                avatar::PaletteEntry::Pattern { width, height, pixels } =>
                                    if let Some(pcol) = pixels.get(((x % height) * width + (y % width)) as usize) {
                                        *pcol
                                    } else {
                                        col
                                    },
                                avatar::PaletteEntry::Animation { delay, width, height, frames } => {
                                    let pixels: &Vec<(u8, u8, u8)> = frames.get(((framecount / delay.max(1)) % frames.len() as i32) as usize).unwrap();
                                    if let Some(pcol) = pixels.get(((x % height) * width + (y % width)) as usize) {
                                        *pcol
                                    } else {
                                        col
                                    }
                                },
                                avatar::PaletteEntry::Video => {
                                    context::PaletteType::from_color(col)
                                        .as_ref().and_then(|pty| players.players.get(pty))
                                        .and_then(|player| Some(player.get_pixel(x, y)))
                                        .unwrap_or(col)
                                },
                            };

                            let wordlen = wordref.chars().count();
                            let mut c1 = wordref.chars().nth(idx % wordlen).unwrap();
                            let mut is_emoji = emojis::get(&format!("{}", c1)).is_some();
                            idx = idx + 1;
                            let mut c2 = wordref.chars().nth(idx % wordlen).unwrap();
                            if emojis::get(&format!("{}", c2)).is_some() { c2 = '.'; }
                            idx = idx + 1;

                            let s = self.get_surrounding(ctx.bgcolor, x, y);
                            if s == 0b01101011 || s == 0b01101111 {
                                c1 = '|'; is_emoji = false;
                            } else if s == 0b11010110 || s == 0b11010110 {
                                if is_emoji { c1 = '.'; }
                                c2 = '|'; is_emoji = false;
                            } else if s == 0b00101011 || s == 0b00101111 || s == 0b00101110 || s == 0b00101100 || s == 0b00001011 {
                                c1 = ' '; c2 = '/'; is_emoji = false;
                            } else if s == 0b10010110 || s == 0b10010111 || s == 0b10010011 || s == 0b10010001 || s == 0b00010110 {
                                c1 = '\\'; c2 = ' '; is_emoji = false;
                            } else if s == 0b00111111 {
                                c1 = '-'; c2 = '/'; is_emoji = false;
                            } else if s == 0b10011111 {
                                c1 = '\\'; c2 = '-'; is_emoji = false;
                            } else if s == 0b00011111 {
                                c1 = '-'; c2 = '-'; is_emoji = false;
                            } else if s == 0b00001111 {
                                c1 = ' '; c2 = '_'; is_emoji = false;
                            } else if s == 0b00010111 {
                                c1 = '_'; c2 = ' '; is_emoji = false;
                            } else if s == 0b00000111 {
                                c1 = '_'; c2 = '_'; is_emoji = false;
                            }

                            if is_emoji {
                                write!(output, "{}", format!("{}", c1).truecolor(r, g, b).on_custom_color(bgcustom)).unwrap();
                            } else {
                                write!(output, "{}", format!("{}{}", c1, c2).truecolor(r, g, b).on_custom_color(bgcustom)).unwrap();
                            }
                        },
                        None => {
                            idx = idx + 1;
                            write!(output, "{}", "  ".to_owned().on_custom_color(bg)).unwrap();
                        },
                    }
                }
                write!(output, "\r\n").unwrap();
            }
            stdout.write(&output).unwrap();
        }
    }

    pub fn render_ascii_classic(&mut self, ctx: &context::Context, framecount: i32, word: &String) {
        let mut stdout = std::io::stdout().into_raw_mode().unwrap();

        let eyeword = "I".to_owned();
        let mouthword = "o".to_owned();
        let shirtword = "#".to_owned();
        let coatword = "COAT".to_owned();

        let bgcustom = colored::customcolors::CustomColor::new(
            ctx.bgcolor.0 as u8 + 1,
            ctx.bgcolor.1 as u8 + 1,
            ctx.bgcolor.2 as u8 + 1,
        );

        if (framecount % 6) == 0 {
            self.populate_pixels();
            print!("\x1b[2J\x1b[1;1H");
            let mut idx = 0;
            for x in 0..(self.dims.w as _) {
                for y in 0..(self.dims.h as _) {
                    match self.get_pixel(ctx.bgcolor, x, y) {
                        Some((r, g, b)) => {
                            let wordref = if g > r && g > b {
                                &eyeword
                            } else if r == g && r == b {
                                &shirtword
                            } else if r < 50 && g < 50 && b < 50 {
                                &coatword
                            } else if r > 210 && g < 180 && b < 180 {
                                &mouthword
                            } else {
                                word
                            };

                            let wordlen = wordref.chars().count();
                            let mut c1 = wordref.chars().nth(idx % wordlen).unwrap();
                            let mut is_emoji = emojis::get(&format!("{}", c1)).is_some();
                            idx = idx + 1;
                            let mut c2 = wordref.chars().nth(idx % wordlen).unwrap();
                            if emojis::get(&format!("{}", c2)).is_some() { c2 = '.'; }
                            idx = idx + 1;

                            let s = self.get_surrounding(ctx.bgcolor, x, y);
                            if s == 0b01101011 || s == 0b01101111 {
                                c1 = '|'; is_emoji = false;
                            } else if s == 0b11010110 || s == 0b11010110 {
                                if is_emoji { c1 = '.'; }
                                c2 = '|'; is_emoji = false;
                            } else if s == 0b00101011 || s == 0b00101111 || s == 0b00101110 || s == 0b00101100 || s == 0b00001011 {
                                c1 = ' '; c2 = '/'; is_emoji = false;
                            } else if s == 0b10010110 || s == 0b10010111 || s == 0b10010011 || s == 0b10010001 || s == 0b00010110 {
                                c1 = '\\'; c2 = ' '; is_emoji = false;
                            } else if s == 0b00111111 {
                                c1 = '-'; c2 = '/'; is_emoji = false;
                            } else if s == 0b10011111 {
                                c1 = '\\'; c2 = '-'; is_emoji = false;
                            } else if s == 0b00011111 {
                                c1 = '-'; c2 = '-'; is_emoji = false;
                            } else if s == 0b00001111 {
                                c1 = ' '; c2 = '_'; is_emoji = false;
                            } else if s == 0b00010111 {
                                c1 = '_'; c2 = ' '; is_emoji = false;
                            } else if s == 0b00000111 {
                                c1 = '_'; c2 = '_'; is_emoji = false;
                            }

                            if is_emoji {
                                write!(stdout, "{}", format!("{}", c1).truecolor(r, g, b).on_custom_color(bgcustom)).unwrap();
                            } else {
                                write!(stdout, "{}", format!("{}{}", c1, c2).truecolor(r, g, b).on_custom_color(bgcustom)).unwrap();
                            }
                        },
                        None => {
                            idx = idx + 1;
                            write!(stdout, "  ").unwrap();
                        },
                    }
                }
                write!(stdout, "\r\n").unwrap();
            }
        }
    }

    pub fn render_ascii_shadow(&mut self, ctx: &context::Context, framecount: i32) {
        let mut stdout = std::io::stdout().into_raw_mode().unwrap();
        let pixels_len = (self.dims.w * self.dims.h * 4.0) as _;
        let mut pixels = vec![0; pixels_len];
        let word = "hcolonw".to_owned();
        let eyeword = "ISEEYOU".to_owned();

        if (framecount % 6) == 0 {
            self.populate_pixels();
            print!("\x1b[2J\x1b[1;1H");
            let mut idx = 0;
            for x in 0..(self.dims.w as _) {
                for y in 0..(self.dims.h as _) {
                    match self.get_pixel(ctx.bgcolor, x, y) {
                        Some((r, g, b)) => {
                            let (wordref, (nr, ng, nb)) = if g > r && g > b {
                                (&eyeword, (255, 0, 0))
                            } else {
                                (&word, (100, 100, 100))
                            };

                            let wordlen = wordref.chars().count();
                            let mut c1 = wordref.chars().nth(idx % wordlen).unwrap();
                            idx = idx + 1;
                            let mut c2 = wordref.chars().nth(idx % wordlen).unwrap();
                            idx = idx + 1;

                            write!(stdout, "{}", format!("{}{}", c1, c2).truecolor(nr, ng, nb)).unwrap();
                        },
                        None => {
                            idx = idx + 1;
                            write!(stdout, "  ").unwrap();
                        },
                    }
                }
                write!(stdout, "\r\n").unwrap();
            }
        }
    }
}

