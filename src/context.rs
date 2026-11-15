use crate::{framebuffer, gl, utils};

use std::{collections::HashMap, env};

use colors_transform::{Color, Rgb};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum PaletteType {
    Hair,
    Eyes,
    Skin,
    Highlight,
    Eyebags,
    PartyHat,
}

impl PaletteType {
    pub fn from_string(nm: &str) -> Option<Self> {
        match nm {
            "hair" => Some(Self::Hair),
            "eyes" => Some(Self::Eyes),
            "skin" => Some(Self::Skin),
            "highlight" => Some(Self::Highlight),
            "eyebags" => Some(Self::Eyebags),
            "hat" => Some(Self::PartyHat),
            _ => None,
        }
    }
    pub fn from_color((r, g, b): (u8, u8, u8)) -> Option<Self> {
        if (186..=188).contains(&r) && (176..=178).contains(&g) && (189..=191).contains(&b) {
            Some(Self::Hair)
        } else if (158..=162).contains(&r) && (148..=152).contains(&g) && (159..=162).contains(&b) {
            Some(Self::Highlight)
        } else if g > r && g > b {
            Some(Self::Eyes)
        } else if (242..=246).contains(&r) && (238..=242).contains(&g) && (234..=238).contains(&b) {
            Some(Self::Skin)
        } else if r == 182 && g == 142 && b == 139 {
            Some(Self::Eyebags)
        } else if r == 255 && g == 0 && b == 0 {
            Some(Self::PartyHat)
        } else {
            None
        }
    }
}

pub struct VideoPlayer<'a> {
    pub fb: framebuffer::Framebuffer,
    pub mpv: &'a libmpv::Mpv,
    pub mpv_render_context: libmpv::render::RenderContext,
    /// 只为把 EventContext 的生命周期绑定在 VideoPlayer 上，代码里不直接读它
    #[allow(dead_code)]
    pub mpv_event_context: libmpv::events::EventContext<'a>,
}

impl<'a> VideoPlayer<'a> {
    /// 失败时返回 None：某些驱动组合下 libmpv 的 GL 渲染上下文创建不出来，
    /// 这时视频填充不可用，但角色渲染应该照常工作
    pub fn new(ctx: &Context) -> Option<Self> {
        let mpv = match libmpv::Mpv::new() {
            Ok(m) => Box::leak(Box::new(m)),
            Err(e) => {
                log::warn!("libmpv 初始化失败：{}，视频填充不可用", e);
                return None;
            }
        };
        // 这些属性失败不影响主功能，失败就跳过
        for (k, v) in [("mute", "yes"), ("keepaspect", "no"), ("loop", "inf")] {
            if let Err(e) = mpv.set_property(k, v) {
                log::debug!("mpv set_property {} 失败: {}", k, e);
            }
        }
        let _ = mpv.set_property("video-timing-offset", 0);
        let _ = mpv.set_property(
            "ytdl-format",
            "bestvideo[height=?144][fps<=?30][vcodec!=?vp9]+bestaudio/best",
        );
        let mpv_render_context = libmpv::render::RenderContext::new(
            unsafe { mpv.ctx.as_mut() },
            vec![
                libmpv::render::RenderParam::ApiType(libmpv::render::RenderParamApiType::OpenGl),
                libmpv::render::RenderParam::InitParams(libmpv::render::OpenGLInitParams {
                    get_proc_address,
                    ctx: &ctx.video,
                }),
            ],
        );
        let mpv_render_context = match mpv_render_context {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "mpv GL 渲染上下文创建失败：{}（显卡驱动/PRIME 下常见），视频填充不可用",
                    e
                );
                return None;
            }
        };
        let mpv_event_context = mpv.create_event_context();
        let fb = framebuffer::Framebuffer::new(ctx, false, (114, 64), (0, 0));

        Some(Self {
            fb,
            mpv,
            mpv_render_context,
            mpv_event_context,
        })
    }

    pub fn update(&mut self, ctx: &Context) {
        self.fb.bind(ctx);
        self.mpv_render_context
            .render::<i32>(
                self.fb.fbo as _,
                self.fb.dims.w as _,
                self.fb.dims.h as _,
                true,
            )
            .unwrap();
        // log::info!(
        //     "hair: {:?} {:?}",
        //     self.mpv.get_property::<String>("path"),
        //     self.mpv.get_property::<f64>("time-remaining"),
        // );
        self.fb.populate_pixels();
    }

    pub fn get_pixel(&self, x: i32, y: i32) -> (u8, u8, u8) {
        let offsetx = (114 - 64) / 2;
        // let offsetx = 0;
        let invx = (self.fb.dims.h as i32) - (x + 1);
        let idx = ((self.fb.dims.w as i32) * invx * 4 + (offsetx + y) * 4) as usize;
        let r = if let Some(r) = self.fb.pixels.get(idx) {
            *r
        } else {
            0
        };
        let g = if let Some(g) = self.fb.pixels.get(idx + 1) {
            *g
        } else {
            0
        };
        let b = if let Some(b) = self.fb.pixels.get(idx + 2) {
            *b
        } else {
            0
        };
        (r, g, b)
    }
}

#[allow(dead_code)]
pub struct Context {
    pub sdl2: sdl2::Sdl,
    pub video: sdl2::VideoSubsystem,
    pub gl_context: sdl2::video::GLContext,
    pub window: sdl2::video::Window,
    pub preview_enabled: bool,
    pub clearcolor: Rgb,
    pub bgcolor: (i32, i32, i32),
    pub dims: utils::Dimensions,
    pub attrib_expressions: HashMap<String, gl::types::GLuint>,
}

pub fn get_proc_address(video: &&sdl2::VideoSubsystem, name: &str) -> *mut std::ffi::c_void {
    video.gl_get_proc_address(name) as *mut std::ffi::c_void
}

impl Context {
    pub fn new() -> Self {
        let sdl2 = sdl2::init().expect("SDL 初始化失败");
        let video = sdl2
            .video()
            .expect("SDL video 子系统初始化失败（没有可用显示？）");
        // sdl2.mouse().set_relative_mouse_mode(true);
        // sdl2::hint::set("SDL_HINT_MOUSE_RELATIVE_MODE_WARP", "1");

        // let gl_attr = video.gl_attr();
        // gl_attr.set_context_major_version(3);
        // gl_attr.set_context_minor_version(2);

        // A stream does not need a presented SDL surface. Hidden/off-screen windows can
        // block swap on Wayland; never use their frame callbacks to clock the renderer.
        let serving = std::env::var("COLONQ_SERVE")
            .ok()
            .and_then(|v| v.trim().parse::<u16>().ok())
            .is_some();
        // 推流模式默认隐藏预览窗口；COLONQ_PREVIEW=1/0 可显式覆盖
        let preview_enabled = utils::env_flag("COLONQ_PREVIEW", !serving);
        let mut builder = video.window("colonq", 640, 360);
        builder.opengl();
        if !preview_enabled {
            builder.hidden();
        }
        let window = builder.build().expect("创建 SDL GL 窗口失败");
        let gl_context = window
            .gl_create_context()
            .expect("创建 OpenGL 上下文失败（显卡驱动/PRIME/会话环境问题）");

        // This must happen AFTER the GL context is current. Previously this call was
        // made before context creation and its failure was silently ignored.
        let vsync = utils::env_flag("COLONQ_VSYNC", false);
        if let Err(e) = video.gl_set_swap_interval(if vsync { 1 } else { 0 }) {
            log::warn!(
                "设置 GL swap interval 失败: {}；推流无预览模式不调用 swap",
                e
            );
        }
        log::info!(
            "SDL 预览: {}；窗口交换: {}",
            if preview_enabled { "显示" } else { "隐藏" },
            if preview_enabled {
                "开启"
            } else {
                "禁用（推流不依赖窗口可见性）"
            }
        );
        gl::load_with(|s| video.gl_get_proc_address(s) as *const std::ffi::c_void);

        // 直接问驱动实际用的是哪个渲染器（llvmpipe = 软件渲染，说明没走显卡）
        unsafe {
            use std::ffi::CStr;
            let get = |e: gl::types::GLenum| {
                let p = gl::GetString(e);
                if p.is_null() {
                    return "?".to_string();
                }
                CStr::from_ptr(p as *const i8)
                    .to_string_lossy()
                    .into_owned()
            };
            log::info!(
                "GL 后端: vendor={} | renderer={} | version={}",
                get(gl::VENDOR),
                get(gl::RENDERER),
                get(gl::VERSION)
            );
        }

        let (winw, winh) = window.size();

        let attrib_expressions: HashMap<String, gl::types::GLuint> = HashMap::from([
            ("happy".to_owned(), 5),
            ("angry".to_owned(), 6),
            ("sad".to_owned(), 7),
            ("relaxed".to_owned(), 8),
            ("surprised".to_owned(), 9),
            ("aa".to_owned(), 10),
            ("ih".to_owned(), 11),
            ("ou".to_owned(), 12),
            ("ee".to_owned(), 13),
            ("oh".to_owned(), 14),
            ("blink".to_owned(), 15),
        ]);

        // 背景色：COLONQ_BGCOLOR=000000 / #000000 之类；留空或写错就退回默认（原作者的深紫）
        let rgb = match env::var("COLONQ_BGCOLOR") {
            Ok(colorstr) => {
                let c = colorstr.trim();
                if c.is_empty() {
                    Rgb::from(0x00 as _, 0x00 as _, 0x00 as _)
                } else {
                    match Rgb::from_hex_str(c) {
                        Ok(rgb) => rgb,
                        Err(_) => {
                            log::warn!("COLONQ_BGCOLOR={:?} 解析失败，用默认背景色", colorstr);
                            Rgb::from(0x00 as _, 0x00 as _, 0x00 as _)
                        }
                    }
                }
            }
            Err(_) => {
                Rgb::from(0x00 as _, 0x00 as _, 0x00 as _) // 默认纯黑（原作者是 0x15050f 深紫）
            }
        };

        unsafe {
            // set unchanging options
            gl::Viewport(0, 0, winw as _, winh as _);
            gl::ClearColor(
                rgb.get_red() / 255.0,
                rgb.get_green() / 255.0,
                rgb.get_blue() / 255.0,
                1.0,
            );
            gl::ClearDepth(1.0);
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::LEQUAL);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            // gl::Enable(gl::CULL_FACE); // NOTE: THIS BREAKS LIBMPV MAKE SURE TO DISABLE IT :3
            // gl::CullFace(gl::FRONT);
        }

        Self {
            sdl2,
            video,
            gl_context,
            window,
            preview_enabled,
            clearcolor: rgb,

            bgcolor: (
                rgb.get_red() as _,
                rgb.get_green() as _,
                rgb.get_blue() as _,
            ),
            dims: utils::Dimensions {
                w: winw as _,
                h: winh as _,
            },
            attrib_expressions,
        }
    }
}
