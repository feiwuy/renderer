pub mod hat;

use crate::{context, fig, gl, shader, utils, vrm};

use std::collections::HashMap;

use image::{AnimationDecoder, ImageDecoder};

type Color = (u8, u8, u8);

#[derive(Clone)]
pub enum PaletteEntry {
    Color(Color),
    Pattern {
        width: i32,
        height: i32,
        pixels: Vec<Color>,
    },
    Animation {
        delay: i32,
        width: i32,
        height: i32,
        frames: Vec<Vec<Color>>,
    },
    Video,
}

impl PaletteEntry {
    pub fn from_image(path: &str) -> Option<Self> {
        // let surface = sdl2::surface::Surface::from_file(path).ok()?;
        // let bytes = surface.without_lock()?;
        // let mut reader = image::io::Reader::open(path).ok()?;
        let mut f = std::fs::File::open(path).ok()?;
        match image::codecs::gif::GifDecoder::new(&mut f) {
            Ok(gifd) => {
                let mut delay = 3;
                let mut frames = Vec::new();
                let (width, height) = gifd.dimensions();
                for rf in gifd.into_frames() {
                    let f = rf.unwrap();
                    let (num, denom) = f.delay().numer_denom_ms();
                    delay = num / (denom * 17);
                    let bytes = f.buffer().clone().into_raw();
                    let pixels = bytes
                        .chunks_exact(4)
                        .map(|c| match c {
                            [_, _, _, 0] => (0, 0, 0),
                            [r, g, b, _] => (*r, *g, *b),
                            _ => (0, 0, 0),
                        })
                        .collect();
                    frames.push(pixels);
                }
                Some(PaletteEntry::Animation {
                    delay: delay as _,
                    width: width as _,
                    height: height as _,
                    frames,
                })
            }
            Err(_) => {
                log::info!("i'm here");
                let img = image::io::Reader::open(path)
                    .ok()?
                    .with_guessed_format()
                    .ok()?
                    .decode()
                    .ok()?;
                log::info!("now i'm there");
                let width = img.width();
                let height = img.height();
                let bytes = img.into_rgba8().into_raw();
                let pixels = bytes
                    .chunks_exact(4)
                    .map(|c| match c {
                        [_, _, _, 0] => (0, 0, 0),
                        [r, g, b, _] => (*r, *g, *b),
                        _ => (0, 0, 0),
                    })
                    .collect();
                Some(Self::Pattern {
                    width: width as _,
                    height: height as _,
                    pixels,
                })
            }
        }
    }
}

pub struct PalettePlayers {
    pub players: HashMap<context::PaletteType, context::VideoPlayer<'static>>,
}

impl PalettePlayers {
    pub fn new(ctx: &context::Context) -> Self {
        // COLONQ_NO_MPV=1：完全不创建 libmpv 实例（每个实例会带若干线程和几十 MB 内存）
        if utils::env_flag("COLONQ_NO_MPV", false) {
            log::info!("COLONQ_NO_MPV：跳过 libmpv 初始化（视频填充不可用，省内存/线程）");
            return Self {
                players: HashMap::new(),
            };
        }
        // 任意一个创建失败都不致命，缺的只是对应部位的视频填充
        // 注意 edition 2018 下数组的 into_iter 会给引用，这里用 Vec
        let players: HashMap<context::PaletteType, context::VideoPlayer> = vec![
            context::PaletteType::Hair,
            context::PaletteType::Skin,
            context::PaletteType::Eyes,
            context::PaletteType::Highlight,
            context::PaletteType::PartyHat,
        ]
        .into_iter()
        .filter_map(|pty| context::VideoPlayer::new(&ctx).map(move |p| (pty, p)))
        .collect();
        Self { players }
    }

    pub fn update(&mut self, ctx: &context::Context, palette: &Palette) {
        for (pty, player) in self.players.iter_mut() {
            if let Some(PaletteEntry::Video) = palette.color_mapping.get(pty) {
                player.mpv.unpause().unwrap();
                player.update(ctx);
            } else {
                player.mpv.pause().unwrap();
            }
        }
    }
}

pub struct Palette {
    pub default_word: String,
    /// 部位编号 -> 用哪套字符（author 的调色板按 Hair/Eyes/Skin/… 分表，这里是通用版）
    pub part_words: HashMap<u8, String>,
    pub word_mapping: HashMap<context::PaletteType, String>,
    pub color_mapping: HashMap<context::PaletteType, PaletteEntry>,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            default_word: "lcolonq".to_owned(),
            part_words: HashMap::new(),
            word_mapping: HashMap::new(),
            color_mapping: HashMap::from([]),
        }
    }

    pub fn lookup(&self, col: Color) -> (bool, &String, PaletteEntry) {
        if let Some(ty) = context::PaletteType::from_color(col) {
            let (custom, word) = if let Some(w) = self.word_mapping.get(&ty) {
                (true, w)
            } else {
                (false, &self.default_word)
            };
            let pal = if let Some(p) = self.color_mapping.get(&ty) {
                p.clone()
            } else {
                PaletteEntry::Color(col)
            };
            (custom, word, pal)
        } else {
            (false, &self.default_word, PaletteEntry::Color(col))
        }
    }
}

pub struct Avatar {
    pub scene: vrm::Scene,
    pub nodes: Vec<vrm::Node>,
    pub position: Box<dyn Fn(&fig::Control) -> glam::Mat4>,
    // pub hats: Vec<hat::Hat>,
    // pub pumpkin: hat::Pumpkin,
}

impl Avatar {
    pub fn new<F>(ctx: &context::Context, path: &str, position: F) -> Self
    where
        F: Fn(&fig::Control) -> glam::Mat4 + 'static,
    {
        let scene = vrm::Scene::new(&ctx, path);
        let nodes = scene.nodes.clone();
        Self {
            scene,
            nodes,
            position: Box::new(position),
            // hats: Vec::new(),
            // pumpkin: hat::Pumpkin::new(&ctx),
        }
    }

    // pub fn add_hat(&mut self, hat: hat::Hat) {
    //     self.hats.push(hat);
    // }

    pub fn transform_bone(&mut self, nm: &str, t: &glam::Mat4) {
        // 模型可能没有某根骨骼（比如非人形模型），此时跳过而不是 panic
        if let Some(ni) = self.scene.bone_node_indices.get(nm) {
            self.nodes[*ni].transform = self.scene.nodes[*ni].transform.mul_mat4(t);
        }
    }

    pub fn render(
        &self,
        ctx: &context::Context,
        shader: &shader::Shader,
        view: &glam::Mat4,
        projection: &glam::Mat4,
        sts: &fig::Control,
    ) {
        let position = (*self.position)(sts);
        let normal_matrix = position.inverse().transpose();
        unsafe {
            gl::UniformMatrix4fv(
                shader.uniform_normal,
                1,
                false as u8,
                normal_matrix.to_cols_array().as_ptr(),
            );
        }
        let global_transforms = self.scene.compute_global_transforms(&self.nodes, &position);
        self.scene.render(ctx, shader, &global_transforms);
        // for h in self.hats.iter() {
        //     h.render(ctx, view, projection, &self, &global_transforms);
        // }
        // self.pumpkin.render(ctx, view, projection, &self, &global_transforms);
    }
}
